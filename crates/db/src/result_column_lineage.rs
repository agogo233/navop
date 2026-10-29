//! SQL 结果列的来源分析。
//!
//! 查询结果本身不携带字段注释，需要在执行后补查表结构元数据。本模块负责从
//! SELECT 语句中推导「每个结果列可能来自哪些基表的哪个字段」，供上层在拿到
//! 表结构后把注释贴回结果列。
//!
//! 设计原则：
//! - 只做保守推导。无法唯一确定的来源返回空或标记 `complete = false`，由上层
//!   退化为按列名唯一匹配，宁缺毋滥。
//! - 支持 join、非递归 CTE、FROM 派生表、列别名和只引用单个字段的表达式。
//! - 递归 CTE、UNION 等复杂结构标记为不完整，避免错误的注释映射。

use std::collections::HashMap;

use sqlparser::ast::{
    self, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, Query, Select, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, TableFactor, TableWithJoins,
};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;

/// 一次 SELECT 的结果列来源分析。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultColumnLineage {
    /// 查询引用到的基表（按首次出现顺序去重）。
    pub tables: Vec<String>,
    /// 顶层结果列来源；通配符条目由调用方按表结构展开成多列。
    pub columns: Vec<ResultColumnSource>,
    /// 分析是否完整。为 `false` 时调用方应退化为按列名唯一匹配。
    pub complete: bool,
}

/// 单个结果列（或通配符）的来源描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultColumnSource {
    /// 结果列引用的源字段。表达式可能包含多个候选；解析后若都指向同一个
    /// 源字段则继承该字段的注释。
    Refs(Vec<ColumnReference>),
    /// 通配符：按该基表的列顺序展开。
    Wildcard { table: usize },
    /// 无法确定来源。
    Unknown,
}

/// 一个字段引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnReference {
    /// 候选基表下标（对应 [`ResultColumnLineage::tables`]）。
    pub tables: Vec<usize>,
    /// 字段名（保留 SQL 中的大小写，匹配时忽略大小写）。
    pub column: String,
}

#[derive(Debug, Clone)]
struct OutputColumn {
    /// 输出列名（别名或列名）；通配符与无别名表达式为 `None`。
    name: Option<String>,
    entry: OutputEntry,
}

#[derive(Debug, Clone)]
enum OutputEntry {
    Refs(Vec<ColumnReference>),
    Wildcard { table: usize },
    Unknown,
}

impl OutputColumn {
    fn to_source(&self) -> ResultColumnSource {
        match &self.entry {
            OutputEntry::Refs(refs) => ResultColumnSource::Refs(refs.clone()),
            OutputEntry::Wildcard { table } => ResultColumnSource::Wildcard { table: *table },
            OutputEntry::Unknown => ResultColumnSource::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
struct Relation {
    /// 源名称（基表原始名或 CTE / 派生表别名）。
    name: String,
    alias: Option<String>,
    kind: RelationKind,
}

#[derive(Debug, Clone)]
enum RelationKind {
    Base(usize),
    Virtual(Vec<OutputColumn>),
    Unknown,
}

impl Relation {
    fn matches_qualifier(&self, qualifier: &str) -> bool {
        if self
            .alias
            .as_deref()
            .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
        {
            return true;
        }
        if !self.name.is_empty() && self.name.eq_ignore_ascii_case(qualifier) {
            return true;
        }
        // 允许用裸表名限定 `schema.table` / `db.table`。
        let bare = self.name.rsplit('.').next().unwrap_or(&self.name);
        !bare.is_empty() && bare.eq_ignore_ascii_case(qualifier)
    }
}

struct Scope {
    relations: Vec<Relation>,
}

impl Scope {
    fn resolve_qualified(&self, qualifier: &str, column: &str) -> Option<Vec<ColumnReference>> {
        let mut matches = self
            .relations
            .iter()
            .filter(|relation| relation.matches_qualifier(qualifier));
        let relation = matches.next()?;
        if matches.next().is_some() {
            // 限定名同时命中多个关系，无法唯一确定。
            return None;
        }

        match &relation.kind {
            RelationKind::Base(table) => Some(vec![ColumnReference {
                tables: vec![*table],
                column: column.to_string(),
            }]),
            RelationKind::Virtual(outputs) => resolve_virtual_output(outputs, column),
            RelationKind::Unknown => None,
        }
    }

    fn resolve_unqualified(&self, column: &str) -> Option<Vec<ColumnReference>> {
        let mut candidate_tables = Vec::new();
        let mut refs = Vec::new();

        for relation in &self.relations {
            match &relation.kind {
                RelationKind::Base(table) => candidate_tables.push(*table),
                RelationKind::Virtual(outputs) => {
                    if outputs
                        .iter()
                        .any(|output| matches!(output.entry, OutputEntry::Unknown))
                    {
                        // 虚拟表存在未知输出，无法排除同名冲突。
                        return None;
                    }
                    let explicit: Vec<&OutputColumn> = outputs
                        .iter()
                        .filter(|output| {
                            output
                                .name
                                .as_deref()
                                .is_some_and(|name| name.eq_ignore_ascii_case(column))
                        })
                        .collect();
                    match explicit.len() {
                        0 => {
                            for output in outputs {
                                if let OutputEntry::Wildcard { table } = output.entry {
                                    candidate_tables.push(table);
                                }
                            }
                        }
                        1 => match &explicit[0].entry {
                            OutputEntry::Refs(explicit_refs) => refs.extend(explicit_refs.clone()),
                            OutputEntry::Wildcard { table } => candidate_tables.push(*table),
                            OutputEntry::Unknown => return None,
                        },
                        _ => return None,
                    }
                }
                RelationKind::Unknown => return None,
            }
        }

        if !candidate_tables.is_empty() {
            candidate_tables.sort_unstable();
            candidate_tables.dedup();
            refs.push(ColumnReference {
                tables: candidate_tables,
                column: column.to_string(),
            });
        }
        (!refs.is_empty()).then_some(refs)
    }
}

fn resolve_virtual_output(outputs: &[OutputColumn], column: &str) -> Option<Vec<ColumnReference>> {
    if outputs
        .iter()
        .any(|output| matches!(output.entry, OutputEntry::Unknown))
    {
        return None;
    }

    let explicit: Vec<&OutputColumn> = outputs
        .iter()
        .filter(|output| {
            output
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(column))
        })
        .collect();
    match explicit.len() {
        1 => match &explicit[0].entry {
            OutputEntry::Refs(refs) => Some(refs.clone()),
            OutputEntry::Wildcard { table } => Some(vec![ColumnReference {
                tables: vec![*table],
                column: column.to_string(),
            }]),
            OutputEntry::Unknown => None,
        },
        0 => {
            let tables: Vec<usize> = outputs
                .iter()
                .filter_map(|output| match output.entry {
                    OutputEntry::Wildcard { table } => Some(table),
                    _ => None,
                })
                .collect();
            (!tables.is_empty()).then_some(vec![ColumnReference {
                tables,
                column: column.to_string(),
            }])
        }
        _ => None,
    }
}

struct Analyzer {
    tables: Vec<String>,
    complete: bool,
}

impl Analyzer {
    fn new() -> Self {
        Self {
            tables: Vec::new(),
            complete: true,
        }
    }

    fn table_index(&mut self, name: &str) -> usize {
        if let Some(index) = self.tables.iter().position(|table| table == name) {
            return index;
        }
        self.tables.push(name.to_string());
        self.tables.len() - 1
    }

    fn analyze_query(
        &mut self,
        query: &Query,
        outer_ctes: &HashMap<String, Vec<OutputColumn>>,
    ) -> Vec<OutputColumn> {
        let mut ctes = outer_ctes.clone();
        if let Some(with) = &query.with {
            if with.recursive {
                // 递归 CTE 的列来源需要不动点推导，保守放弃。
                self.complete = false;
            }
            for cte in &with.cte_tables {
                if !cte.alias.columns.is_empty() {
                    self.complete = false;
                }
                let name = cte.alias.name.value.to_ascii_lowercase();
                let outputs = self.analyze_query(&cte.query, &ctes);
                ctes.insert(name, outputs);
            }
        }

        match &*query.body {
            SetExpr::Select(select) => self.analyze_select(select, &ctes),
            SetExpr::Query(query) => self.analyze_query(query, &ctes),
            SetExpr::SetOperation { left, .. } => {
                // UNION / EXCEPT / INTERSECT 的列名与顺序以左侧为准，但两侧
                // 来源可能不同，交由按列名唯一匹配的兜底逻辑处理。
                self.complete = false;
                match &**left {
                    SetExpr::Select(select) => self.analyze_select(select, &ctes),
                    SetExpr::Query(query) => self.analyze_query(query, &ctes),
                    _ => Vec::new(),
                }
            }
            _ => {
                self.complete = false;
                Vec::new()
            }
        }
    }

    fn analyze_select(
        &mut self,
        select: &Select,
        ctes: &HashMap<String, Vec<OutputColumn>>,
    ) -> Vec<OutputColumn> {
        if select.exclude.is_some() {
            self.complete = false;
        }

        let scope = Scope {
            relations: self.build_relations(&select.from, ctes),
        };

        let mut outputs = Vec::new();
        for item in &select.projection {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    outputs.push(output_for_expr(expr, None, &scope));
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    outputs.push(output_for_expr(expr, Some(alias.value.clone()), &scope));
                }
                SelectItem::ExprWithAliases { expr, aliases } => {
                    // 多别名会把一个表达式拆成多列，列数无法与结果对齐。
                    self.complete = false;
                    outputs.push(output_for_expr(
                        expr,
                        aliases.first().map(|alias| alias.value.clone()),
                        &scope,
                    ));
                }
                SelectItem::Wildcard(options) => {
                    if !wildcard_options_are_plain(options) {
                        self.complete = false;
                    }
                    self.expand_wildcard(None, &scope, &mut outputs);
                }
                SelectItem::QualifiedWildcard(kind, options) => {
                    if !wildcard_options_are_plain(options) {
                        self.complete = false;
                    }
                    match kind {
                        SelectItemQualifiedWildcardKind::ObjectName(name) => {
                            let qualifier = object_name_to_string(name).unwrap_or_default();
                            self.expand_wildcard(Some(&qualifier), &scope, &mut outputs);
                        }
                        SelectItemQualifiedWildcardKind::Expr(_) => {
                            self.complete = false;
                            outputs.push(OutputColumn {
                                name: None,
                                entry: OutputEntry::Unknown,
                            });
                        }
                    }
                }
            }
        }
        outputs
    }

    fn build_relations(
        &mut self,
        from: &[TableWithJoins],
        ctes: &HashMap<String, Vec<OutputColumn>>,
    ) -> Vec<Relation> {
        let mut relations = Vec::new();
        for table_with_joins in from {
            self.push_table_factor(&table_with_joins.relation, ctes, &mut relations);
            for join in &table_with_joins.joins {
                self.push_table_factor(&join.relation, ctes, &mut relations);
            }
        }
        relations
    }

    fn push_table_factor(
        &mut self,
        factor: &TableFactor,
        ctes: &HashMap<String, Vec<OutputColumn>>,
        relations: &mut Vec<Relation>,
    ) {
        match factor {
            TableFactor::Table { name, alias, .. } => {
                let Some(raw_name) = object_name_to_string(name) else {
                    self.complete = false;
                    relations.push(unknown_relation());
                    return;
                };
                let alias_name = alias.as_ref().map(|alias| alias.name.value.clone());
                if alias
                    .as_ref()
                    .is_some_and(|alias| !alias.columns.is_empty())
                {
                    // `AS t(a, b)` 会重命名输出列，按列名无法继续推导。
                    self.complete = false;
                }

                if let Some(outputs) = ctes.get(&raw_name.to_ascii_lowercase()) {
                    relations.push(Relation {
                        name: raw_name.clone(),
                        alias: alias_name,
                        kind: RelationKind::Virtual(outputs.clone()),
                    });
                } else {
                    let table = self.table_index(&raw_name);
                    relations.push(Relation {
                        name: raw_name,
                        alias: alias_name,
                        kind: RelationKind::Base(table),
                    });
                }
            }
            TableFactor::Derived { subquery, alias, .. } => {
                if alias
                    .as_ref()
                    .is_some_and(|alias| !alias.columns.is_empty())
                {
                    self.complete = false;
                }
                let outputs = self.analyze_query(subquery, ctes);
                let alias_name = alias.as_ref().map(|alias| alias.name.value.clone());
                relations.push(Relation {
                    name: alias_name.clone().unwrap_or_default(),
                    alias: alias_name,
                    kind: RelationKind::Virtual(outputs),
                });
            }
            TableFactor::NestedJoin {
                table_with_joins,
                alias,
            } => {
                if alias.is_some() {
                    self.complete = false;
                }
                self.push_table_factor(&table_with_joins.relation, ctes, relations);
                for join in &table_with_joins.joins {
                    self.push_table_factor(&join.relation, ctes, relations);
                }
            }
            _ => {
                // 表函数、UNNEST、JSON_TABLE 等无法从表结构推导列。
                self.complete = false;
                relations.push(unknown_relation());
            }
        }
    }

    fn expand_wildcard(&mut self, qualifier: Option<&str>, scope: &Scope, outputs: &mut Vec<OutputColumn>) {
        let relations: Vec<&Relation> = match qualifier {
            None => {
                if scope.relations.is_empty() {
                    self.complete = false;
                    outputs.push(OutputColumn {
                        name: None,
                        entry: OutputEntry::Unknown,
                    });
                    return;
                }
                scope.relations.iter().collect()
            }
            Some(qualifier) => {
                let matches: Vec<&Relation> = scope
                    .relations
                    .iter()
                    .filter(|relation| relation.matches_qualifier(qualifier))
                    .collect();
                if matches.len() != 1 {
                    self.complete = false;
                    outputs.push(OutputColumn {
                        name: None,
                        entry: OutputEntry::Unknown,
                    });
                    return;
                }
                matches
            }
        };

        for relation in relations {
            match &relation.kind {
                RelationKind::Base(table) => outputs.push(OutputColumn {
                    name: None,
                    entry: OutputEntry::Wildcard { table: *table },
                }),
                RelationKind::Virtual(outputs_of_relation) => {
                    outputs.extend(outputs_of_relation.iter().cloned());
                }
                RelationKind::Unknown => {
                    self.complete = false;
                    outputs.push(OutputColumn {
                        name: None,
                        entry: OutputEntry::Unknown,
                    });
                }
            }
        }
    }
}

fn unknown_relation() -> Relation {
    Relation {
        name: String::new(),
        alias: None,
        kind: RelationKind::Unknown,
    }
}

fn output_for_expr(expr: &Expr, alias: Option<String>, scope: &Scope) -> OutputColumn {
    let name = alias.or_else(|| match expr {
        Expr::Identifier(ident) => Some(ident.value.clone()),
        Expr::CompoundIdentifier(parts) => parts.last().map(|ident| ident.value.clone()),
        _ => None,
    });

    let mut refs = Vec::new();
    if collect_expr_refs(expr, scope, &mut refs) {
        OutputColumn {
            name,
            entry: OutputEntry::Refs(refs),
        }
    } else {
        OutputColumn {
            name,
            entry: OutputEntry::Unknown,
        }
    }
}

fn collect_expr_refs(expr: &Expr, scope: &Scope, out: &mut Vec<ColumnReference>) -> bool {
    match expr {
        Expr::Identifier(ident) => match scope.resolve_unqualified(&ident.value) {
            Some(refs) => {
                out.extend(refs);
                true
            }
            None => false,
        },
        Expr::CompoundIdentifier(parts) if parts.len() >= 2 => {
            let Some(column) = parts.last().map(|ident| ident.value.as_str()) else {
                return false;
            };
            let qualifier = parts[..parts.len() - 1]
                .iter()
                .map(|part| part.value.as_str())
                .collect::<Vec<_>>()
                .join(".");
            match scope.resolve_qualified(&qualifier, column) {
                Some(refs) => {
                    out.extend(refs);
                    true
                }
                None => false,
            }
        }
        Expr::CompoundIdentifier(_) => false,
        Expr::Nested(inner)
        | Expr::UnaryOp { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::Extract { expr: inner, .. }
        | Expr::Collate { expr: inner, .. }
        | Expr::Ceil { expr: inner, .. }
        | Expr::Floor { expr: inner, .. }
        | Expr::Convert { expr: inner, .. }
        | Expr::Prefixed { value: inner, .. }
        | Expr::JsonAccess { value: inner, .. }
        | Expr::CompoundFieldAccess { root: inner, .. }
        | Expr::Named { expr: inner, .. }
        | Expr::IsFalse(inner)
        | Expr::IsNotFalse(inner)
        | Expr::IsTrue(inner)
        | Expr::IsNotTrue(inner)
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner)
        | Expr::IsUnknown(inner)
        | Expr::IsNotUnknown(inner) => collect_expr_refs(inner, scope, out),
        Expr::BinaryOp { left, right, .. }
        | Expr::IsDistinctFrom(left, right)
        | Expr::IsNotDistinctFrom(left, right)
        | Expr::AnyOp { left, right, .. }
        | Expr::AllOp { left, right, .. }
        | Expr::AtTimeZone {
            timestamp: left,
            time_zone: right,
        } => {
            collect_expr_refs(left, scope, out) && collect_expr_refs(right, scope, out)
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            collect_expr_refs(expr, scope, out)
                && collect_expr_refs(low, scope, out)
                && collect_expr_refs(high, scope, out)
        }
        Expr::InList { expr, list, .. } => {
            collect_expr_refs(expr, scope, out)
                && list
                    .iter()
                    .all(|item| collect_expr_refs(item, scope, out))
        }
        Expr::InUnnest {
            expr, array_expr, ..
        } => {
            collect_expr_refs(expr, scope, out)
                && collect_expr_refs(array_expr, scope, out)
        }
        Expr::InSubquery { expr, .. } => {
            // 子查询内部的列属于另一个作用域，不参与外层结果列来源。
            collect_expr_refs(expr, scope, out)
        }
        Expr::Like { expr, pattern, .. }
        | Expr::ILike { expr, pattern, .. }
        | Expr::SimilarTo { expr, pattern, .. }
        | Expr::RLike { expr, pattern, .. } => {
            collect_expr_refs(expr, scope, out)
                && collect_expr_refs(pattern, scope, out)
        }
        Expr::IsNormalized { expr, .. } => collect_expr_refs(expr, scope, out),
        Expr::Substring {
            expr,
            substring_from,
            substring_for,
            ..
        } => {
            collect_expr_refs(expr, scope, out)
                && substring_from
                    .as_deref()
                    .map(|item| collect_expr_refs(item, scope, out))
                    .unwrap_or(true)
                && substring_for
                    .as_deref()
                    .map(|item| collect_expr_refs(item, scope, out))
                    .unwrap_or(true)
        }
        Expr::Trim {
            expr,
            trim_what,
            trim_characters,
            ..
        } => {
            collect_expr_refs(expr, scope, out)
                && trim_what
                    .as_deref()
                    .map(|item| collect_expr_refs(item, scope, out))
                    .unwrap_or(true)
                && trim_characters
                    .as_deref()
                    .map(|items| {
                        items
                            .iter()
                            .all(|item| collect_expr_refs(item, scope, out))
                    })
                    .unwrap_or(true)
        }
        Expr::Position { expr, r#in } => {
            collect_expr_refs(expr, scope, out) && collect_expr_refs(r#in, scope, out)
        }
        Expr::Overlay {
            expr,
            overlay_what,
            overlay_from,
            overlay_for,
        } => {
            collect_expr_refs(expr, scope, out)
                && collect_expr_refs(overlay_what, scope, out)
                && collect_expr_refs(overlay_from, scope, out)
                && overlay_for
                    .as_deref()
                    .map(|item| collect_expr_refs(item, scope, out))
                    .unwrap_or(true)
        }
        Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            let mut ok = true;
            if let Some(operand) = operand {
                ok &= collect_expr_refs(operand, scope, out);
            }
            for condition in conditions {
                ok &= collect_expr_refs(&condition.condition, scope, out);
                ok &= collect_expr_refs(&condition.result, scope, out);
            }
            if let Some(else_result) = else_result {
                ok &= collect_expr_refs(else_result, scope, out);
            }
            ok
        }
        Expr::Function(function) => collect_function_refs(&function.args, scope, out),
        Expr::Tuple(items) => items
            .iter()
            .all(|item| collect_expr_refs(item, scope, out)),
        Expr::GroupingSets(sets) | Expr::Cube(sets) | Expr::Rollup(sets) => sets
            .iter()
            .flatten()
            .all(|item| collect_expr_refs(item, scope, out)),
        // 常量、子查询、通配符等不产生外层字段引用。
        Expr::Value(_)
        | Expr::TypedString(_)
        | Expr::Interval(_)
        | Expr::Wildcard(_)
        | Expr::QualifiedWildcard(..)
        | Expr::Subquery(_)
        | Expr::Exists { .. } => true,
        _ => false,
    }
}

fn collect_function_refs(
    args: &FunctionArguments,
    scope: &Scope,
    out: &mut Vec<ColumnReference>,
) -> bool {
    match args {
        FunctionArguments::None => true,
        FunctionArguments::Subquery(_) => true,
        FunctionArguments::List(list) => list.args.iter().all(|arg| match arg {
            FunctionArg::Unnamed(arg) => collect_function_arg_refs(arg, scope, out),
            FunctionArg::Named { arg, .. } | FunctionArg::ExprNamed { arg, .. } => {
                collect_function_arg_refs(arg, scope, out)
            }
        }),
    }
}

fn collect_function_arg_refs(
    arg: &FunctionArgExpr,
    scope: &Scope,
    out: &mut Vec<ColumnReference>,
) -> bool {
    match arg {
        FunctionArgExpr::Expr(expr) => collect_expr_refs(expr, scope, out),
        // `COUNT(*)` 等通配符参数不指向具体字段。
        FunctionArgExpr::QualifiedWildcard(_)
        | FunctionArgExpr::Wildcard
        | FunctionArgExpr::WildcardWithOptions(_) => true,
    }
}

fn wildcard_options_are_plain(options: &ast::WildcardAdditionalOptions) -> bool {
    options.opt_ilike.is_none()
        && options.opt_exclude.is_none()
        && options.opt_except.is_none()
        && options.opt_replace.is_none()
        && options.opt_rename.is_none()
}

fn object_name_to_string(name: &ast::ObjectName) -> Option<String> {
    let parts = name
        .0
        .iter()
        .map(|part| part.as_ident().map(|ident| ident.value.as_str()))
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join("."))
}

/// 解析 SQL 并推导结果列来源。解析失败或非 SELECT 语句返回 `None`。
pub(crate) fn analyze_sql_lineage(
    dialect: &dyn Dialect,
    sql: &str,
) -> Option<ResultColumnLineage> {
    let statements = Parser::parse_sql(dialect, sql).ok()?;
    let statement = statements.first()?;
    let ast::Statement::Query(query) = statement else {
        return None;
    };

    let mut analyzer = Analyzer::new();
    let outputs = analyzer.analyze_query(query, &HashMap::new());
    Some(ResultColumnLineage {
        tables: analyzer.tables,
        columns: outputs.iter().map(OutputColumn::to_source).collect(),
        complete: analyzer.complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::dialect::GenericDialect;

    fn analyze(sql: &str) -> ResultColumnLineage {
        analyze_sql_lineage(&GenericDialect {}, sql).expect("query should parse")
    }

    fn refs(tables: Vec<usize>, column: &str) -> ResultColumnSource {
        ResultColumnSource::Refs(vec![ColumnReference {
            tables,
            column: column.to_string(),
        }])
    }

    #[test]
    fn resolves_join_with_qualified_columns_and_alias() {
        let lineage = analyze(
            "SELECT u.id AS user_id, o.amount FROM users u JOIN orders o ON u.id = o.user_id",
        );
        assert_eq!(lineage.tables, vec!["users", "orders"]);
        assert!(lineage.complete);
        assert_eq!(
            lineage.columns,
            vec![refs(vec![0], "id"), refs(vec![1], "amount")]
        );
    }

    #[test]
    fn expands_qualified_wildcard_to_source_table() {
        let lineage = analyze("SELECT u.* FROM users u JOIN orders o ON u.id = o.user_id");
        assert_eq!(lineage.tables, vec!["users", "orders"]);
        assert_eq!(lineage.columns, vec![ResultColumnSource::Wildcard { table: 0 }]);
    }

    #[test]
    fn keeps_unqualified_column_ambiguous_across_joined_tables() {
        let lineage = analyze("SELECT id FROM users u JOIN orders o ON u.id = o.user_id");
        assert_eq!(lineage.columns, vec![refs(vec![0, 1], "id")]);
    }

    #[test]
    fn follows_non_recursive_cte() {
        let lineage = analyze(
            "WITH recent AS (SELECT id, name AS label FROM users) \
             SELECT label FROM recent",
        );
        assert_eq!(lineage.tables, vec!["users"]);
        assert!(lineage.complete);
        assert_eq!(lineage.columns, vec![refs(vec![0], "name")]);
    }

    #[test]
    fn follows_derived_table_alias() {
        let lineage = analyze(
            "SELECT t.n FROM (SELECT name AS n FROM users) t",
        );
        assert_eq!(lineage.tables, vec!["users"]);
        assert_eq!(lineage.columns, vec![refs(vec![0], "name")]);
    }

    #[test]
    fn single_field_function_inherits_source_column() {
        let lineage = analyze("SELECT UPPER(name) FROM users");
        assert_eq!(lineage.columns, vec![refs(vec![0], "name")]);
    }

    #[test]
    fn multi_field_expression_keeps_all_references() {
        let lineage = analyze("SELECT price * qty FROM items");
        assert_eq!(lineage.columns.len(), 1);
        match &lineage.columns[0] {
            ResultColumnSource::Refs(refs) => {
                assert_eq!(refs.len(), 2);
                assert_eq!(refs[0].column, "price");
                assert_eq!(refs[1].column, "qty");
            }
            other => panic!("expected refs, got {other:?}"),
        }
    }

    #[test]
    fn union_marks_lineage_incomplete() {
        let lineage = analyze("SELECT id FROM users UNION SELECT id FROM orders");
        assert!(!lineage.complete);
        assert_eq!(lineage.tables, vec!["users"]);
    }

    #[test]
    fn recursive_cte_marks_lineage_incomplete() {
        let lineage = analyze(
            "WITH RECURSIVE tree AS (SELECT id FROM nodes) SELECT id FROM tree",
        );
        assert!(!lineage.complete);
    }
}
