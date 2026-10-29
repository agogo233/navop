//! Auto 模式粘性会话的 schema 对齐真机验证（issue #433）。
//!
//! Auto 模式（编辑器默认事务模式）复用一条粘性会话跨语句执行（issue #312），
//! 并在建立会话后调用 `switch_session_schema` 把连接对齐到编辑器选中的 schema
//! （`SqlEditorTab::prepare_auto_session`）。这里验证该契约的底层事实：
//! `execute_session` 只发 SQL 本身、不会重放 schema 切换，未限定表名的查询只有
//! 在对齐之后才解析到目标 schema，否则会落到服务端默认 search_path（通常是
//! public）——这正是 #433 的故障表面。

use db::executor::{ExecOptions, QueryResult, SqlResult};
use db::manager::GlobalDbState;

use crate::real_databases::common::env::{optional_database, postgres_config, skip_database};

#[tokio::test]
async fn postgres_real_sticky_session_resolves_unqualified_tables_in_selected_schema() {
    let Some(config) = postgres_config() else {
        skip_database(
            "PostgreSQL sticky schema",
            "ONETCLI_TEST_POSTGRES_PASSWORD (and optionally host/port/user)",
        );
        return;
    };
    let config = optional_database(&config, "postgres");
    let connection_id = config.id.clone();
    let schema = format!("navop_real_pg_sticky_{}", std::process::id());

    let mut state = GlobalDbState::new();
    state.register_connection(config);

    let session_id = state
        .create_session_direct(connection_id.clone(), None)
        .await
        .expect("sticky session should connect");

    // 1. fixture：目标 schema 与 public 各建一张同名表，值不同，用来区分
    //    “未限定表名”究竟解析到了哪个 schema。
    let setup = format!(
        "DROP SCHEMA IF EXISTS \"{schema}\" CASCADE;\n\
         CREATE SCHEMA \"{schema}\";\n\
         CREATE TABLE \"{schema}\".navop_sticky_probe (id INT);\n\
         INSERT INTO \"{schema}\".navop_sticky_probe VALUES (42);\n\
         DROP TABLE IF EXISTS public.navop_sticky_probe;\n\
         CREATE TABLE public.navop_sticky_probe (id INT);\n\
         INSERT INTO public.navop_sticky_probe VALUES (7);"
    );
    execute_script(&state, &session_id, &setup).await;

    // 2. 对齐 schema 之前（等价 #433 修复前的行为）：未限定表名落到 public。
    let fallback = session_query(&state, &session_id, "SELECT id FROM navop_sticky_probe").await;
    assert_cell(&fallback, 0, 0, "7");

    // 3. 对齐 schema 之后：同一条会话、同样未限定表名，解析到目标 schema。
    state
        .switch_session_schema(session_id.clone(), schema.clone())
        .await
        .expect("session should switch to the selected schema");
    let scoped = session_query(&state, &session_id, "SELECT id FROM navop_sticky_probe").await;
    assert_cell(&scoped, 0, 0, "42");

    // 4. 会话级设置跨执行存活（粘性 = 同一条物理连接），后续执行仍在目标 schema。
    let again = session_query(&state, &session_id, "SELECT id FROM navop_sticky_probe").await;
    assert_cell(&again, 0, 0, "42");

    // 5. 切回 public：证明切换是会话级 search_path，而不是查询改写。
    state
        .switch_session_schema(session_id.clone(), "public".to_string())
        .await
        .expect("session should switch back to public");
    let back = session_query(&state, &session_id, "SELECT id FROM navop_sticky_probe").await;
    assert_cell(&back, 0, 0, "7");

    // 6. 收尾：清理 fixture 并关闭会话。
    execute_script(
        &state,
        &session_id,
        &format!(
            "DROP SCHEMA IF EXISTS \"{schema}\" CASCADE;\n\
             DROP TABLE IF EXISTS public.navop_sticky_probe;"
        ),
    )
    .await;
    state
        .close_session_direct(&session_id)
        .await
        .expect("sticky session should close");
}

async fn execute_script(state: &GlobalDbState, session_id: &str, sql: &str) {
    let results = state
        .execute_session(
            session_id.to_string(),
            sql.to_string(),
            Some(ExecOptions::default()),
        )
        .await
        .expect("session script should execute");
    for result in &results {
        if let SqlResult::Error(error) = result {
            panic!("session script failed: {}\n{sql}", error.message);
        }
    }
}

async fn session_query(state: &GlobalDbState, session_id: &str, sql: &str) -> QueryResult {
    let results = state
        .execute_session(session_id.to_string(), sql.to_string(), None)
        .await
        .expect("session query should execute");
    for result in &results {
        if let SqlResult::Error(error) = result {
            panic!("session query failed: {}\n{sql}", error.message);
        }
    }
    let SqlResult::Query(query) = results.into_iter().next().unwrap_or_else(|| {
        panic!("session query should return a result: {sql}");
    }) else {
        panic!("session query should return rows: {sql}");
    };
    query
}

fn assert_cell(result: &QueryResult, row: usize, column: usize, expected: &str) {
    let view = result.typed_view().expect("query result should be valid");
    match view.cell(row, column).expect("cell should exist") {
        db::executor::QueryCellRef::Text(value) => assert_eq!(value, expected),
        other => panic!("expected text at ({row},{column}), got {other:?}"),
    }
}
