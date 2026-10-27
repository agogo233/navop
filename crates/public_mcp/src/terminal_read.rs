use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalReadRequest {
    pub target: String,
    pub lines: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalReadResult {
    pub target: String,
    pub text: String,
    pub requested_lines: usize,
    pub returned_lines: usize,
    pub available_lines: usize,
    pub history_size: usize,
    pub screen_lines: usize,
    pub columns: usize,
    pub truncated: bool,
    /// 全屏程序（vim、less 等）占据屏幕时为 true，此时 text 是它的界面。
    pub alt_screen: bool,
    /// 用户往回滚的行数；非 0 表示正在看历史。
    pub display_offset: usize,
    /// 光标在 text 里的行号（0 起）。
    pub cursor_row: usize,
    /// 光标列号（0 起）。
    pub cursor_column: usize,
    /// 用户当前选中的文本；没有选区时为 None。
    pub selection: Option<String>,
}
