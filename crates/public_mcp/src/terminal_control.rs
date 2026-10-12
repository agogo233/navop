use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalControlAction {
    Interrupt,
    /// 注入运行时 shell integration，恢复嵌套 shell（sudo -i / su）丢失的 OSC 事件流。
    Reintegrate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalControlReadiness {
    SubmissionPending,
    CommandRunning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalControlRequest {
    pub target: String,
    pub action: TerminalControlAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalControlResult {
    pub target: String,
    pub action: TerminalControlAction,
    pub sent: bool,
    /// 仅 `Interrupt` 有意义；`Reintegrate` 恒为 `None`。
    pub readiness_before: Option<TerminalControlReadiness>,
}
