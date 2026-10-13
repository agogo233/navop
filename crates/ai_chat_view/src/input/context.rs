//! 输入框上下文视图模型(纯展示)。
//!
//! 这些类型刻意**只用 `SharedString`、不依赖 `agent_runtime` / 任何业务 crate**,
//! 让 [`AgentInput`](super::AgentInput) 保持"哑组件":它只接收一份
//! [`AgentComposerContext`],具体数据由上层(集成视图)按连接类型映射并注入。
//!
//! 设计要点:
//! - 本地 One agent 只摆「模型 + 权限档位」,ACP 那份另摆 agent 自己声明的东西 ——
//!   两边共享同一份结构体,各取所需字段;
//! - 模型下拉项用 [`ComposerModelOption`] 表达,避免从展示文案反解析 provider/model;
//! - 工具执行模式的下拉项用 [`ComposerMenuOption`] 表达。

use gpui::SharedString;
use rust_i18n::t;

use crate::usage::ContextUsage;

/// 底部模型选择器的当前值(本轮执行参数)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposerModel {
    pub provider: SharedString,
    pub model: SharedString,
    /// true 时标签只显示模型名，隐藏 provider 前缀（ACP 的 provider 是 agent 名，
    /// 工具栏已单独展示 agent，再拼一遍是重复）。
    pub model_only: bool,
}

impl ComposerModel {
    pub fn new(provider: impl Into<SharedString>, model: impl Into<SharedString>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            model_only: false,
        }
    }

    pub fn with_model_only(mut self) -> Self {
        self.model_only = true;
        self
    }
}

/// 底部模型选择器的结构化选项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerModelOption {
    /// 选项标识(emit 选择事件时回传)。
    pub id: SharedString,
    /// provider 的稳定标识,用于重建运行时。
    pub provider_id: SharedString,
    /// provider 展示名。
    pub provider_label: SharedString,
    /// 模型名,原样传给模型客户端。
    pub model: SharedString,
    /// 次要说明(可选,显示在标签下方)。
    pub hint: Option<SharedString>,
    /// true 时标签只显示模型名，不拼 provider 前缀（ACP 模型用）。
    pub model_only: bool,
}

impl ComposerModelOption {
    pub fn new(
        id: impl Into<SharedString>,
        provider_id: impl Into<SharedString>,
        provider_label: impl Into<SharedString>,
        model: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            provider_id: provider_id.into(),
            provider_label: provider_label.into(),
            model: model.into(),
            hint: None,
            model_only: false,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_model_only(mut self) -> Self {
        self.model_only = true;
        self
    }

    pub fn display_label(&self) -> SharedString {
        if self.model_only {
            self.model.clone()
        } else {
            SharedString::from(format!("{} / {}", self.provider_label, self.model))
        }
    }

    pub fn to_composer_model(&self) -> ComposerModel {
        let model = ComposerModel::new(self.provider_label.clone(), self.model.clone());
        if self.model_only {
            model.with_model_only()
        } else {
            model
        }
    }
}

/// 内置下拉菜单的一个选项(模型 / 工具模式 / 任务模式共用)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerMenuOption {
    /// 选项标识(emit 选择事件时回传)。
    pub id: SharedString,
    /// 展示文案。
    pub label: SharedString,
    /// 次要说明(可选,显示在标签下方)。
    pub hint: Option<SharedString>,
}

impl ComposerMenuOption {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// 面板抬头「Agent」切换器中的一个可选执行后端。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerAgentOption {
    /// `None` 表示内置 Agent;`Some(id)` 表示外部 ACP Agent。
    pub id: Option<SharedString>,
    pub label: SharedString,
    pub subtitle: SharedString,
    pub selected: bool,
    pub connecting: bool,
    pub enabled: bool,
}

impl ComposerAgentOption {
    pub fn local(label: impl Into<SharedString>, selected: bool, connecting: bool) -> Self {
        Self {
            id: None,
            label: label.into(),
            subtitle: SharedString::from(t!("AgentUi.built_in_agent").to_string()),
            selected,
            connecting,
            enabled: true,
        }
    }

    pub fn acp(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        selected: bool,
        connecting: bool,
    ) -> Self {
        Self {
            id: Some(id.into()),
            label: label.into(),
            subtitle: SharedString::from("ACP Agent"),
            selected,
            connecting,
            enabled: true,
        }
    }

    pub fn invalid_acp(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        diagnostic: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: Some(id.into()),
            label: label.into(),
            subtitle: diagnostic.into(),
            selected: false,
            connecting: false,
            enabled: false,
        }
    }

    pub fn element_id(&self) -> SharedString {
        match &self.id {
            Some(id) => SharedString::from(format!("agent-option-acp-{id}")),
            None => SharedString::from("agent-option-local"),
        }
    }
}

/// 底部上下文栏的「工作区」信息。
///
/// 只承载展示所需的文本；真实路径与切换动作由上层持有。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposerWorkspaceInfo {
    /// 显示名(通常是目录名)；空串表示尚无工作区。
    pub label: SharedString,
    /// 完整路径，用于下拉项副标题与 tooltip。
    pub path: Option<SharedString>,
    /// 是否为 Git 仓库；决定分支 / Worktree 两个入口是否可用。
    pub is_git_repo: bool,
}

impl ComposerWorkspaceInfo {
    pub fn new(label: impl Into<SharedString>, path: Option<impl Into<SharedString>>) -> Self {
        Self {
            label: label.into(),
            path: path.map(Into::into),
            is_git_repo: false,
        }
    }

    pub fn with_git_repo(mut self, is_git_repo: bool) -> Self {
        self.is_git_repo = is_git_repo;
        self
    }
}

/// 底部上下文栏「工作区」下拉的一项。
///
/// 候选来自宿主已经打开过的工作区集合；与左侧会话导航里的那个下拉同源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerWorkspaceOption {
    pub label: SharedString,
    /// 工作区根目录(绝对路径)，选中后回传给宿主。
    pub path: SharedString,
    pub current: bool,
}

impl ComposerWorkspaceOption {
    pub fn new(
        label: impl Into<SharedString>,
        path: impl Into<SharedString>,
        current: bool,
    ) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            current,
        }
    }

    /// 下拉菜单项的稳定标识。
    pub fn element_id(&self) -> SharedString {
        SharedString::from(format!("composer-workspace-{}", self.path))
    }
}

/// 底部上下文栏「分支」下拉的一项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerBranchOption {
    pub name: SharedString,
    pub current: bool,
    /// 远程分支(`origin/xxx`)。选中远程分支由上层决定是否切成本地跟踪分支。
    pub remote: bool,
}

impl ComposerBranchOption {
    pub fn new(name: impl Into<SharedString>, current: bool, remote: bool) -> Self {
        Self {
            name: name.into(),
            current,
            remote,
        }
    }

    /// 下拉菜单项的稳定标识。
    pub fn element_id(&self) -> SharedString {
        SharedString::from(format!(
            "composer-branch-{}{}",
            if self.remote { "remote-" } else { "" },
            self.name
        ))
    }
}

/// 底部上下文栏的「Worktree」状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposerWorktreeState {
    /// 当前会话是否跑在独立 worktree 上(复选框勾选态)。
    pub enabled: bool,
    /// 当前 worktree 名；`enabled` 时用于 chip 文案。
    pub label: Option<SharedString>,
    /// 已勾选但**尚未创建** —— 真正的创建推迟到本会话第一次发送
    /// (见 [`AgentChatView::submit_with_window`](super::AgentChatView))，
    /// 免得只是勾一下就在磁盘上留下 worktree、还把工作区根切走。
    ///
    /// `pending` 时没有 `label`(名字要等创建时才知道)，chip 显示通用文案。
    pub pending: bool,
}

impl ComposerWorktreeState {
    pub fn new(enabled: bool, label: Option<impl Into<SharedString>>) -> Self {
        Self {
            enabled,
            label: label.map(Into::into),
            pending: false,
        }
    }

    /// 已勾选、等首次对话时创建。
    pub fn pending() -> Self {
        Self {
            enabled: true,
            label: None,
            pending: true,
        }
    }
}

/// 注入给输入框的整体上下文(只读展示)。
///
/// 由上层构造并通过 [`AgentInput::set_context`](super::AgentInput::set_context) 注入。
#[derive(Clone, Debug, Default)]
pub struct AgentComposerContext {
    /// ACP 会话标题(agent 报上来的会话名);空字符串时抬头用占位文案。
    pub session_title: SharedString,
    /// 右侧能力标签。
    pub capabilities: Vec<SharedString>,
    /// 当前模型(底部高亮 chip);`None` 时显示「选择模型」。
    pub model: Option<ComposerModel>,
    /// 工具执行模式当前文案(如 `自动`)。
    pub execution_mode_label: SharedString,
    /// 底部上下文栏：当前工作区。
    pub workspace: ComposerWorkspaceInfo,
    /// 底部上下文栏：工作区下拉的候选(空表示宿主尚未注入)。
    pub workspace_options: Vec<ComposerWorkspaceOption>,
    /// 底部上下文栏：可切换的分支列表(空表示非 Git 仓库或数据未就绪)。
    pub branch_options: Vec<ComposerBranchOption>,
    /// 底部上下文栏：Worktree 状态。
    pub worktree: ComposerWorktreeState,
    /// 上下文用量读数(本地/ACP 同源);`None` 时不画圆环。
    pub context_usage: Option<ContextUsage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_option_hint_is_optional() {
        let plain = ComposerMenuOption::new("a", "Alpha");
        assert!(plain.hint.is_none());
        let hinted = ComposerMenuOption::new("b", "Beta").with_hint("快");
        assert_eq!(hinted.hint, Some(SharedString::from("快")));
    }

    #[test]
    fn default_context_is_empty() {
        let ctx = AgentComposerContext::default();
        assert!(ctx.model.is_none());
        assert!(ctx.execution_mode_label.is_empty());
    }

    #[test]
    fn model_option_keeps_provider_and_model_structured() {
        let option = ComposerModelOption::new("openai:gpt-4.1", "openai", "OpenAI", "gpt-4.1")
            .with_hint("默认");

        assert_eq!(option.id.as_ref(), "openai:gpt-4.1");
        assert_eq!(option.provider_id.as_ref(), "openai");
        assert_eq!(option.provider_label.as_ref(), "OpenAI");
        assert_eq!(option.model.as_ref(), "gpt-4.1");
        assert_eq!(option.display_label().as_ref(), "OpenAI / gpt-4.1");
        assert_eq!(
            option.to_composer_model(),
            ComposerModel::new("OpenAI", "gpt-4.1")
        );
    }

    #[test]
    fn model_only_option_hides_the_provider_prefix() {
        // ACP：provider 是 agent 名，标签只用模型名。
        let option = ComposerModelOption::new(
            "acp:opencode-acp:model:9router/deepseek-v4-flash",
            "opencode-acp",
            "opencode-acp.opencode-acp",
            "9router/deepseek-v4-flash",
        )
        .with_model_only();

        assert_eq!(option.display_label().as_ref(), "9router/deepseek-v4-flash");
        let model = option.to_composer_model();
        assert!(model.model_only);
        assert_eq!(model.provider.as_ref(), "opencode-acp.opencode-acp");
        assert_eq!(model.model.as_ref(), "9router/deepseek-v4-flash");
    }
}
