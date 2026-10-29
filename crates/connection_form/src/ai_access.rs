//! 连接级 AI 访问策略（issue #437）的通用组件。
//!
//! 所有连接表单（数据库 / SSH / Redis / MongoDB / middleware 声明式表单）展示的是同一组
//! 选项（允许 / 只读 / 禁止），因此这里同时提供：
//!
//! - `ai_access_selector`——三档选择器本体，表单只交出一个 `Entity<AiAccessLevel>`；
//! - `ai_access_label` / `level_label` / `level_hint`——文案与说明；
//! - `stored_level` / `write_level`——从连接读取、写回连接（缺省即「允许」）。
//!
//! 新增连接表单时不要复制单选钮布局，直接用 `ai_access_selector`。

use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, SharedString, Styled, div};
use gpui_component::{ActiveTheme, h_flex, radio::Radio, v_flex};
use rust_i18n::t;
use tool_runtime::AiAccessLevel;

/// 读取连接上的级别；连接未落库或存储不可用（如主密钥未就绪）时按默认「允许」处理。
pub fn stored_level(connection_id: Option<i64>, cx: &App) -> AiAccessLevel {
    let Some(connection_id) = connection_id else {
        return AiAccessLevel::Allowed;
    };
    let Some(storage) = cx
        .try_global::<one_core::storage::GlobalStorageState>()
        .map(|state| state.storage.clone())
    else {
        return AiAccessLevel::Allowed;
    };
    storage
        .get::<one_core::storage::ConnectionRepository>()
        .and_then(|repo| repo.ai_access_level(connection_id).ok())
        .unwrap_or_default()
}

/// 把级别写回连接（写「允许」等价于默认值，存储层会存 NULL）。
///
/// 连接本身已经保存成功后调用；失败时上层要把错误显示给用户，
/// 不能让「限制没生效」被静默吞掉。
pub fn write_level(
    repo: &one_core::storage::ConnectionRepository,
    connection_id: i64,
    level: AiAccessLevel,
) -> Result<(), String> {
    repo.set_ai_access_level(connection_id, level)
        .map_err(|error| error.to_string())
}

/// 级别在表单里的短标签。
pub fn level_label(level: AiAccessLevel) -> String {
    match level {
        AiAccessLevel::Allowed => t!("ConnectionForm.ai_access_allowed").to_string(),
        AiAccessLevel::ReadOnly => t!("ConnectionForm.ai_access_read_only").to_string(),
        AiAccessLevel::Denied => t!("ConnectionForm.ai_access_denied").to_string(),
    }
}

/// 级别的一句话说明，显示在选项下方，让用户不用猜「只读」「禁止」的边界。
pub fn level_hint(level: AiAccessLevel) -> String {
    match level {
        AiAccessLevel::Allowed => t!("ConnectionForm.ai_access_allowed_hint").to_string(),
        AiAccessLevel::ReadOnly => t!("ConnectionForm.ai_access_read_only_hint").to_string(),
        AiAccessLevel::Denied => t!("ConnectionForm.ai_access_denied_hint").to_string(),
    }
}

/// 表单里按固定顺序（允许 → 只读 → 禁止）展示所有级别。
pub fn all_levels() -> impl Iterator<Item = AiAccessLevel> {
    AiAccessLevel::ALL.into_iter()
}

/// 单选钮元素 id：同一表单里唯一，且带上级别便于测试断言。
pub fn level_option_id(level: AiAccessLevel) -> SharedString {
    format!("ai-access-{}", level.as_str()).into()
}

/// 把级别写入策略实体，并触发一次重绘。
pub fn apply_level(entity: &Entity<AiAccessLevel>, level: AiAccessLevel, cx: &mut App) {
    entity.update(cx, |current, cx| {
        if *current == level {
            return;
        }
        *current = level;
        cx.notify();
    });
}

/// 组件读出实体里的当前级别。
pub fn selected_level(entity: &Entity<AiAccessLevel>, cx: &App) -> AiAccessLevel {
    *entity.read(cx)
}

/// 表单里这一项的标题，和选项文案放在一起，避免各表单各写一份 i18n key。
pub fn ai_access_label() -> String {
    t!("ConnectionForm.ai_access").to_string()
}

/// 连接已落库、只有「AI 访问策略」没写进去时的提示文案。
///
/// 各连接表单都从这里取文案：`t!` 只查**调用方 crate** 的语言包，同一句译文若要每个表单
/// 各写一份 key，就会漏掉其中一两个（曾因此把 `ConnectionForm.ai_access` 原样显示到界面上）。
/// 保存失败的类型在各表单并不统一（`String` / `anyhow::Error`），所以按 `Display` 接收。
pub fn ai_access_save_failed(error: impl std::fmt::Display) -> String {
    t!("ConnectionForm.ai_access_save_failed", error = error).to_string()
}

/// 「AI 访问」三档选择器：所有连接表单共用的通用组件。
///
/// 表单只持有一个 `Entity<AiAccessLevel>`，渲染时把实体交给本组件；选项顺序、文案、
/// 选中状态与说明都由组件统一维护。新增连接表单时不要再复制一遍单选钮布局。
pub fn ai_access_selector(entity: &Entity<AiAccessLevel>, cx: &App) -> AnyElement {
    let selected = selected_level(entity, cx);
    let entity = entity.clone();
    v_flex()
        .w_full()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .flex_wrap()
                .gap_3()
                .children(all_levels().map(move |level| {
                    let entity = entity.clone();
                    h_flex()
                        .items_center()
                        .gap_1()
                        .flex_shrink_0()
                        .child(
                            Radio::new(level_option_id(level))
                                .label(level_label(level))
                                .checked(selected == level)
                                .on_click(move |_, _, cx| apply_level(&entity, level, cx)),
                        )
                        .into_any_element()
                })),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(level_hint(selected)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::AppContext;

    use super::*;

    #[test]
    fn all_levels_follow_the_allowed_read_only_denied_order() {
        assert_eq!(
            all_levels().collect::<Vec<_>>(),
            vec![
                AiAccessLevel::Allowed,
                AiAccessLevel::ReadOnly,
                AiAccessLevel::Denied
            ]
        );
    }

    #[gpui::test]
    fn without_a_connection_id_the_level_is_allowed(cx: &mut gpui::TestAppContext) {
        // 新建连接还没有 id，回填时必须落到默认级别，而不是 fail-closed。
        let level = cx.update(|cx| stored_level(None, cx));
        assert_eq!(level, AiAccessLevel::Allowed);
    }

    #[test]
    fn every_level_has_a_distinct_option_id_and_non_empty_label() {
        let mut ids = Vec::new();
        for level in all_levels() {
            assert!(!level_label(level).is_empty());
            assert!(!level_hint(level).is_empty());
            let id = level_option_id(level);
            assert!(!ids.contains(&id), "选项 id 必须唯一: {id}");
            ids.push(id);
        }
        assert_eq!(ids.len(), 3);
    }

    #[gpui::test]
    fn apply_level_writes_the_entity(cx: &mut gpui::TestAppContext) {
        let entity = cx.update(|cx| cx.new(|_| AiAccessLevel::Allowed));
        cx.update(|cx| apply_level(&entity, AiAccessLevel::Denied, cx));
        assert_eq!(
            cx.read(|cx| *entity.read(cx)),
            AiAccessLevel::Denied,
            "apply_level 必须把级别写回实体"
        );
        cx.update(|cx| apply_level(&entity, AiAccessLevel::ReadOnly, cx));
        assert_eq!(cx.read(|cx| *entity.read(cx)), AiAccessLevel::ReadOnly);
    }

    #[gpui::test]
    fn selector_shows_the_level_from_the_entity(cx: &mut gpui::TestAppContext) {
        for level in all_levels() {
            let entity = cx.update(|cx| cx.new(|_| level));
            assert_eq!(
                cx.read(|cx| selected_level(&entity, cx)),
                level,
                "选择器必须展示实体里的级别，而不是固定的默认值"
            );
        }
    }
}
