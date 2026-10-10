//! 补全 provider：`@` 提及（本地）与 `/` 命令（ACP agent 推送）各自可挂。
//!
//! gpui-component 的编辑器只挂一个 provider，而这两套触发条件互斥（`@` 出现在词首、
//! `/` 出现在整条输入开头），所以在这一层合并而不是往各自的 provider 里塞对方的逻辑。
//!
//! 两份来源属于**两个后端**：`@` 提及是本地工作区文件，`/` 命令是 ACP agent 声明的
//! 能力。用两个构造器把它们分开——本地那份输入框挂 mentions，ACP 那份挂 slash，
//! 谁都不必背对方的空壳（空壳还会让 `/` 弹出一个永远没有选项的菜单）。

use anyhow::Result;
use gpui::{App, Task, Window};
use gpui_component::Rope;
use gpui_component::input::CompletionProvider;
use lsp_types::{CompletionContext, CompletionResponse};

use crate::input::mention::{MentionCompletionProvider, MentionItem};
use crate::input::slash::{SlashCommandItem, SlashCompletionProvider};

/// 输入框实际挂载的补全 provider。
pub(crate) struct ComposerCompletionProvider {
    mentions: Option<MentionCompletionProvider>,
    slash: Option<SlashCompletionProvider>,
}

impl ComposerCompletionProvider {
    /// 只挂 `@` 提及：本地 One agent 的输入框（本地没有 `/` 命令这回事）。
    pub(crate) fn mentions(mentions: Vec<MentionItem>) -> Self {
        Self {
            mentions: Some(MentionCompletionProvider::new(mentions)),
            slash: None,
        }
    }

    /// 只挂 `/` 命令：ACP agent 的输入框（agent 推送的 `available_commands`）。
    pub(crate) fn slash(commands: Vec<SlashCommandItem>) -> Self {
        Self {
            mentions: None,
            slash: Some(SlashCompletionProvider::new(commands)),
        }
    }
}

impl CompletionProvider for ComposerCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        trigger: CompletionContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        // 斜杠命令形态更严格（必须在输入开头），所以它先判：命中就只给命令，
        // 不会掉进提及分支。
        let text = rope.to_string();
        if let Some(slash) = self.slash.as_ref()
            && SlashCompletionProvider::extract_slash_query(&text, offset).is_some()
        {
            return slash.completions(rope, offset, trigger, window, cx);
        }
        match self.mentions.as_ref() {
            Some(mentions) => mentions.completions(rope, offset, trigger, window, cx),
            None => Task::ready(Ok(CompletionResponse::Array(vec![]))),
        }
    }

    fn is_completion_trigger(&self, offset: usize, new_text: &str, cx: &mut App) -> bool {
        self.slash
            .as_ref()
            .is_some_and(|slash| slash.is_completion_trigger(offset, new_text, cx))
            || self
                .mentions
                .as_ref()
                .is_some_and(|mentions| mentions.is_completion_trigger(offset, new_text, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    /// 两份补全来源属于两个后端，谁也拿不到对方那条通路。
    #[gpui::test]
    fn each_composer_only_has_its_own_completion_channel(cx: &mut TestAppContext) {
        // 本地那份：有 `@` 提及，没有 `/` 命令——本地运行时没有命令表，
        // 挂一条空的 slash 通路只会让用户打了 `/` 却弹出空菜单。
        let local = ComposerCompletionProvider::mentions(Vec::new());
        cx.update(|cx| {
            assert!(
                !local.is_completion_trigger(1, "/", cx),
                "本地输入框不该因为 `/` 弹菜单"
            );
        });

        // ACP 那份：有 `/` 命令（agent 声明的），没有 `@` 提及。
        let acp = ComposerCompletionProvider::slash(Vec::new());
        cx.update(|cx| {
            assert!(
                acp.is_completion_trigger(1, "/", cx),
                "ACP 输入框的 `/` 仍要触发命令菜单"
            );
        });
    }
}
