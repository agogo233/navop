use crate::theme::{AgentChatTheme, resolve_agent_chat_theme, themed_markdown};
use crate::{ChatMessageUIGeneric, MessageExtension};
use gpui::prelude::FluentBuilder;
use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, Window, div};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use one_assets::IconName;
use rust_i18n::t;
use std::cell::Cell;

thread_local! {
    /// 渲染「当前段」的推理时临时置位。
    ///
    /// 推理块默认只在流式期间展开，流一停就收起。可过程区是一段一段推进的：
    /// 每来一个工具调用或下一段推理，上一段就塌下去、又把高度撑回来，用户看到
    /// 的就是一直闪。当前段是用户正在读的那一段，渲染它的时候置位，让推理块保持
    /// 展开；更早的段落照旧收起（分段，而不是整轮不折叠）。
    static FORCE_OPEN_REASONING: Cell<bool> = const { Cell::new(false) };
}

/// 在「当前段」里渲染：进程内的推理块不随流结束自行收起。
pub(crate) fn with_reasoning_force_open<R>(force: bool, render: impl FnOnce() -> R) -> R {
    FORCE_OPEN_REASONING.with(|flag| {
        let previous = flag.replace(force);
        let rendered = render();
        flag.set(previous);
        rendered
    })
}

fn reasoning_force_open() -> bool {
    FORCE_OPEN_REASONING.with(|flag| flag.get())
}

pub fn render_reasoning_block<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    theme: Option<&AgentChatTheme>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = resolve_agent_chat_theme(theme, cx);
    let state_id = SharedString::from(format!("reasoning-expanded-{}", msg.id));
    let expanded_state = window.use_keyed_state(state_id, cx, |_, _| {
        msg.is_reasoning_expanded && !msg.is_streaming
    });
    let is_expanded = reasoning_is_expanded(
        msg.is_streaming,
        *expanded_state.read(cx),
        reasoning_force_open(),
    );

    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .pl_2()
        .border_l_2()
        .border_color(theme.quote_border.opacity(0.7))
        .child(reasoning_header(
            msg,
            is_expanded,
            expanded_state,
            &theme,
            cx,
        ))
        .when(is_expanded, |this| this.child(reasoning_body(msg, &theme)))
        .into_any_element()
}

fn reasoning_header<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    is_expanded: bool,
    expanded_state: gpui::Entity<bool>,
    theme: &AgentChatTheme,
    _cx: &mut App,
) -> AnyElement {
    let icon = if is_expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let tooltip = if is_expanded {
        t!("AgentUi.collapse_reasoning").to_string()
    } else {
        t!("AgentUi.expand_reasoning").to_string()
    };

    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_1()
        .child(
            Button::new(SharedString::from(format!("reasoning-toggle-{}", msg.id)))
                .ghost()
                .xsmall()
                .icon(icon)
                .disabled(msg.is_streaming)
                .tooltip(tooltip)
                .on_click(move |_, _, cx| {
                    expanded_state.update(cx, |expanded, cx| {
                        *expanded = !*expanded;
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(t!("AgentUi.reasoning_process").to_string()),
        )
        .into_any_element()
}

/// 推理块展开态。
///
/// - 正在流式：一定展开（用户要看实时思考）。
/// - 用户手动点开：尊重选择。
/// - 落在过程区「当前段」里：保持展开 —— 这一段还在推进，塌下去只会闪。
fn reasoning_is_expanded(is_streaming: bool, is_user_expanded: bool, force_open: bool) -> bool {
    is_streaming || is_user_expanded || force_open
}

fn reasoning_body<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    theme: &AgentChatTheme,
) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .pl_6()
        .pr_2()
        .pb_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(
            themed_markdown(
                SharedString::from(format!("reasoning-msg-{}", msg.id)),
                msg.reasoning_content.clone(),
                theme,
            )
            .text_xs()
            .selectable(true),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::reasoning_is_expanded;

    #[test]
    fn streaming_reasoning_expands_without_persisting_completed_state() {
        assert!(reasoning_is_expanded(true, false, false));
        assert!(!reasoning_is_expanded(false, false, false));
        assert!(reasoning_is_expanded(false, true, false));
    }

    /// 当前段里的推理：流结束了也不收，直到这一段成为上一段。
    #[test]
    fn live_segment_reasoning_stays_open_after_the_stream_stops() {
        assert!(reasoning_is_expanded(false, false, true));
        assert!(reasoning_is_expanded(true, false, true));
    }
}
