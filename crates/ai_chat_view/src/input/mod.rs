//! Agent 输入框模块:顶部目标 Context Bar + 多行输入 + `@` 提及 + 图片附件 + 底部执行参数工具栏。

mod acp_input;
mod agent_input;
mod attachment;
mod completion;
pub(crate) mod composer_chrome;
mod context;
mod history;
mod mention;
mod model_picker;
mod skill;
mod slash;

pub use acp_input::AcpAgentInput;
pub use agent_input::{AgentInput, AgentInputEvent, QueuedPromptPreview};
pub use attachment::ImageAttachment;
pub(crate) use attachment::prepare_input_images;
pub use context::{
    AgentComposerContext, ComposerAgentOption, ComposerBranchOption, ComposerMenuOption,
    ComposerModel, ComposerModelOption, ComposerPlanItem, ComposerResourcePoolItem,
    ComposerResourcePoolSummary, ComposerResourceSourceOption, ComposerResourceTypeFilter,
    ComposerScope, ComposerSubAgentItem, ComposerTarget, ComposerWorkspaceInfo,
    ComposerWorkspaceOption, ComposerWorktreeState,
};
pub(crate) use history::PromptHistory;
pub use mention::{MentionCompletionProvider, MentionItem};
pub use skill::{ComposerSkillItem, ComposerSkillSummary};
pub use slash::SlashCommandItem;
