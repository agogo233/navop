//! 断开态 Ctrl+D 关闭标签页的行为规则（MobaXterm / Xshell 习惯）。

use super::Keystroke;
use crate::view::text_input::{
    DisconnectedCtrlDCloseTarget, disconnected_ctrl_d_close_target, is_disconnected_ctrl_d,
};
use crate::view::workspace_support::TerminalRenderMode;

fn keystroke(shortcut: &str) -> Keystroke {
    Keystroke::parse_with_os(shortcut, one_core::keybindings::current_os()).expect("解析快捷键失败")
}

#[test]
fn ctrl_d_is_only_detected_without_extra_modifiers() {
    assert!(is_disconnected_ctrl_d(&keystroke("ctrl-d")));
    // 带其它修饰键的 Ctrl+D 不当作"关闭"处理，避免抢占 shell 组合键。
    assert!(!is_disconnected_ctrl_d(&keystroke("ctrl-alt-d")));
    assert!(!is_disconnected_ctrl_d(&keystroke("ctrl-shift-d")));
    // 无修饰键的 d 属于普通输入，必须继续透传。
    assert!(!is_disconnected_ctrl_d(&keystroke("d")));
    assert!(!is_disconnected_ctrl_d(&keystroke("ctrl-e")));
}

#[test]
fn ctrl_d_closes_tab_inside_workspace_and_window_elsewhere() {
    assert_eq!(
        disconnected_ctrl_d_close_target(TerminalRenderMode::WorkspacePane),
        DisconnectedCtrlDCloseTarget::Tab
    );
    assert_eq!(
        disconnected_ctrl_d_close_target(TerminalRenderMode::Embedded),
        DisconnectedCtrlDCloseTarget::Window
    );
}

#[test]
fn disconnected_ctrl_d_keeps_session_handling_untouched() {
    let source = include_str!("../text_input.rs");
    // 会话存活时必须继续把 Ctrl+D 透传成 EOF，不能被关闭逻辑截走：
    // 关键前提是"会话未激活"，而 `accepts_live_terminal_input` 对已断开的 SSH 仍为 true，
    // 所以必须叠加 `terminal_is_disconnected`，否则第二次 Ctrl+D 永远命中不到关闭分支。
    assert!(source.contains(
        "let session_inactive =\n            self.terminal_is_disconnected(cx) || !self.accepts_live_terminal_input(cx);"
    ));
    assert!(
        source.contains(
            "if !is_locked && session_inactive && is_disconnected_ctrl_d(&event.keystroke)"
        )
    );
    assert!(source.contains("is_disconnected_ctrl_d(&event.keystroke)"));
    assert!(source.contains("DisconnectedCtrlDCloseTarget::Tab =>"));
    assert!(source.contains("DisconnectedCtrlDCloseTarget::Window =>"));
    assert!(source.contains("one_core::window_close::request_close_window"));
    // 关闭标签页必须走标签容器的正常关闭生命周期，而不是直接关窗口。
    let tab_branch = source
        .split("DisconnectedCtrlDCloseTarget::Tab =>")
        .nth(1)
        .expect("存在关闭标签页分支")
        .split("DisconnectedCtrlDCloseTarget::Window =>")
        .next()
        .unwrap();
    assert!(tab_branch.contains("TerminalPaneEvent::ClosePaneRequested"));
    assert!(!tab_branch.contains("request_close_window"));
}

#[test]
fn workspace_pane_close_request_respects_split_panes() {
    let source = include_str!("../../workspace/actions.rs");
    let branch = source
        .split("TerminalPaneEvent::ClosePaneRequested =>")
        .nth(1)
        .expect("工作区响应 pane 关闭请求")
        .split("TerminalPaneEvent::")
        .next()
        .unwrap();
    // 单 pane：关整个标签页（走标签容器关闭生命周期）；多 pane：只关当前 pane。
    assert!(branch.contains("this.panes.len() <= 1"));
    assert!(branch.contains("cx.emit(TabContentEvent::CloseRequested)"));
    assert!(branch.contains("this.request_close_pane(pane_id, window, cx)"));
}
