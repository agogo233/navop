mod actions;
mod close;
mod layout;
mod pane_tab_transfer;
mod pane_tool;
mod render;
mod resize;
mod split_model;
mod split_sizing;
mod tab_content;
mod tab_drag;
#[cfg(test)]
mod tests;
mod view;

pub use split_model::{TerminalPaneId, TerminalSplitId, TerminalSplitNode, TerminalSplitTree};
pub use view::{TerminalWorkspace, TerminalWorkspaceEvent};
