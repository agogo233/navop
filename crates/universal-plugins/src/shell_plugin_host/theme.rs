//! `navop.theme` host 模块：宿主的**组件主题**投影给脚本。
//!
//! 与 gpui-shell 内建的 `cx.theme()` 分工不同，两者**都要用**：
//!
//! | 来源 | 内容 | 名字 |
//! |---|---|---|
//! | `cx.theme().colors` | gpui-base 的 18 个语义色（`ColorTokens`） | 中立词汇：`surface`、`destructive`… |
//! | `navop.theme` | 宿主实际在用的 `gpui_component::Theme::colors` | `ThemeColor` 的字段名：一百多个 |
//!
//! 之所以要第二条：宿主界面真正在用的面 —— `list_active` / `table_hover` /
//! `list_even` / `table_even` / `sidebar_*` / `tab_*` / `chart_*` / `success` /
//! `warning` —— 都不在 `ColorTokens` 里，脚本从 `cx.theme()` 读是 `undefined`，
//! 而 `undefined` 进颜色桥会**整页 blank**（`expected a string, got nil`）。
//!
//! ⚠️ **两套命名不是一套**，别照搬：组件主题里没有 `surface` / `destructive`，
//! 对应的是 `popover` / `danger`。用 `names()` 确认，别猜。
//!
//! 颜色一律以 `#rrggbbaa` 返回（`gpui::Hsla` 的序列化形式）：元素样式走的
//! `Bridged::as_color` 与组件 `Badge.color` 走的 `try_parse_color` 都认这个形状。
//!
//! 快照与 `revision()` 的用法见 gpui-shell 的 `theme_tokens.rs`：序列化结果不变
//! 就不加版本号，页面据此决定要不要重建缓存。

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::App;
use gpui_component::Theme as ComponentTheme;
use gpui_shell::{HostError, HostModule, HostObject, HostValue, with_current_app};

/// 不在任何 host 调用栈上时（例如装配期）的报错文案。
const NO_HOST_CALL: &str = "navop.theme needs a live host call";

thread_local! {
    static CACHE: RefCell<ThemeCache> = RefCell::new(ThemeCache::default());
}

#[derive(Default)]
struct ThemeCache {
    /// 上一次序列化出来的颜色表 + 明暗，只用来判断"变没变"。
    key: Option<String>,
    revision: u32,
    snapshot: Option<Rc<ThemeSnapshot>>,
}

/// 一次主题投影：页面读到的全部内容都在这里。
struct ThemeSnapshot {
    revision: u32,
    dark: bool,
    colors: Vec<(String, String)>,
    by_name: HashMap<String, String>,
}

impl Default for ThemeSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            dark: false,
            colors: Vec::new(),
            by_name: HashMap::new(),
        }
    }
}

impl ThemeSnapshot {
    fn get(&self, name: &str) -> Option<&str> {
        self.by_name.get(name).map(String::as_str)
    }
}

/// 当前主题快照，缓存到序列化结果变化为止。
fn snapshot(cx: &App) -> Rc<ThemeSnapshot> {
    // `Theme` 是宿主自己的 Global，未初始化(测试台、极早期)时不该 panic：
    // 空表会让页面自己兜底，而不是把宿主拖下来。
    let Some(theme) = cx.try_global::<ComponentTheme>() else {
        return Rc::new(ThemeSnapshot::default());
    };
    let dark = theme.mode.is_dark();
    let serialized = serde_json::to_value(&theme.colors).unwrap_or(serde_json::Value::Null);
    let colors = collect_colors(&serialized);
    let source = serialized.to_string();
    // 明暗进键：同一份颜色在两个模式下不可能都成立，但真出现时也不该串缓存。
    let mut key = String::with_capacity(source.len() + 1);
    key.push(if dark { 'd' } else { 'l' });
    key.push_str(&source);

    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(snapshot) = &cache.snapshot
            && cache.key.as_deref() == Some(key.as_str())
        {
            return Rc::clone(snapshot);
        }
        let revision = cache.revision.wrapping_add(1);
        let by_name = colors.iter().cloned().collect();
        let snapshot = Rc::new(ThemeSnapshot {
            revision,
            dark,
            colors,
            by_name,
        });
        cache.key = Some(key);
        cache.revision = revision;
        cache.snapshot = Some(Rc::clone(&snapshot));
        snapshot
    })
}

/// 序列化出来的颜色表 → `(名字, "#rrggbbaa")`。
///
/// 靠 serde 反射而不是手抄一份匹配表：字段改名、加色都在组件库里发生，
/// 抄一份就多一处会漂的清单。非字符串字段（组件库将来若有）直接跳过。
fn collect_colors(serialized: &serde_json::Value) -> Vec<(String, String)> {
    let Some(fields) = serialized.as_object() else {
        return Vec::new();
    };
    fields
        .iter()
        .filter_map(|(name, value)| value.as_str().map(|color| (name.clone(), color.to_owned())))
        .collect()
}

fn with_theme<R>(
    read: impl FnOnce(&ThemeSnapshot) -> Result<R, HostError>,
) -> Result<R, HostError> {
    with_current_app(|cx| {
        let snapshot = snapshot(cx);
        read(&snapshot)
    })
    .ok_or_else(|| HostError::new(NO_HOST_CALL))?
}

/// 装配 `navop.theme`。函数体延迟取 `App`，装配期无需 host 栈。
pub(super) fn theme_module() -> HostModule {
    HostModule::new("navop.theme")
        .declarations(
            r#"
            /** `#rrggbbaa`（`gpui::Hsla` 的序列化形式）。 */
            export type ThemeColorValue = string;
            export function appearance(): "light" | "dark";
            export function names(): string[];
            export function colors(): Record<string, ThemeColorValue>;
            export function color(name: string): ThemeColorValue | null;
            /** 主题每次变化 +1；序列化结果不变时不加。 */
            export function revision(): number;
            "#,
        )
        .function("appearance", |_| {
            with_theme(|snapshot| {
                Ok(HostValue::Str(
                    if snapshot.dark { "dark" } else { "light" }.to_owned(),
                ))
            })
        })
        .function("names", |_| {
            with_theme(|snapshot| {
                Ok(HostValue::Array(
                    snapshot
                        .colors
                        .iter()
                        .map(|(name, _)| HostValue::Str(name.clone()))
                        .collect(),
                ))
            })
        })
        .function("colors", |_| {
            with_theme(|snapshot| {
                let mut object = HostObject::new();
                for (name, color) in &snapshot.colors {
                    object = object.field(name.clone(), color.clone());
                }
                Ok(object.into())
            })
        })
        .function("color", |arguments| {
            let name = arguments.string(0)?;
            with_theme(|snapshot| {
                Ok(snapshot
                    .get(&name)
                    .map(|color| HostValue::Str(color.to_owned()))
                    .unwrap_or(HostValue::Null))
            })
        })
        .function("revision", |_| {
            with_theme(|snapshot| Ok(HostValue::Number(f64::from(snapshot.revision))))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_colors() -> Vec<(String, String)> {
        let serialized =
            serde_json::to_value(gpui_component::ThemeColor::default()).expect("serialize");
        collect_colors(&serialized)
    }

    /// `ThemeColor` 的每个字段都必须是颜色；反射靠序列化，序列化失败的字段
    /// 会在页面上表现为"名字查不到"，比整页 blank 难查，所以在宿主侧就断言掉。
    #[test]
    fn every_theme_color_field_is_serialized_as_a_color() {
        let colors = default_colors();

        assert!(
            colors.len() > 100,
            "`ThemeColor` 应该有上百个字段，实际拿到 {}",
            colors.len()
        );
        for (name, color) in &colors {
            assert!(
                color.starts_with('#') && color.len() == 9,
                "`{name}` 不是 #rrggbbaa：{color}"
            );
        }
        assert!(colors.iter().any(|(name, _)| name == "list_active"));
        assert!(colors.iter().any(|(name, _)| name == "table_hover"));
    }

    /// 两套命名本来就不一样，这条测试把它写下来：谁要"顺手统一"会红。
    #[test]
    fn component_theme_names_differ_from_the_semantic_palette() {
        let names: std::collections::HashSet<String> =
            default_colors().into_iter().map(|(name, _)| name).collect();

        assert!(
            names.contains("popover"),
            "语义色的 surface 在组件主题里叫 popover"
        );
        assert!(
            names.contains("danger"),
            "语义色的 destructive 在组件主题里叫 danger"
        );
        assert!(!names.contains("surface"));
        assert!(!names.contains("destructive"));
    }
}
