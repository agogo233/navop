//! 两份输入框的挂载点与按后端路由。
//!
//! 输入区不是一个"懂两种后端"的组件，而是**两份各自独立的组件**：
//! - [`AgentInput`]：本地运行时的 One agent 输入框（计划 / 子代理 / 上下文资源 /
//!   Skill 四张能力面板、资源池、权限档位都在这里）；
//! - [`AcpAgentInput`]：ACP agent 的输入框，只摆 agent 自己声明的东西（会话模式、
//!   模型、`/` 命令）与必需的工作区归属。
//!
//! 两条通路共用的是**事件枚举**（[`AgentInputEvent`]）与**数据类型**（附件、草稿、
//! 队列预览、视图上下文），不是组件实现：ACP 侧改观感、加交互都不会波及 One agent。
//!
//! [`Self::composer`] 是唯一的"当前该用哪一份"判据——判据只有后端，没有别的开关，
//! 所以"切到 ACP agent 才变"这件事在代码里只有一处真相。

use super::*;
use crate::input::{AcpAgentInput, ImageAttachment};

/// 当前生效的输入框实体。
#[derive(Clone)]
pub(super) enum ComposerSlot {
    One(Entity<AgentInput>),
    Acp(Entity<AcpAgentInput>),
}

impl ComposerSlot {
    pub(super) fn set_context(
        &self,
        context: AgentComposerContext,
        cx: &mut Context<AgentChatView>,
    ) {
        match self {
            Self::One(entity) => entity.update(cx, |input, cx| input.set_context(context, cx)),
            Self::Acp(entity) => entity.update(cx, |input, cx| input.set_context(context, cx)),
        }
    }

    pub(super) fn set_queued_submissions(
        &self,
        previews: Vec<QueuedPromptPreview>,
        cx: &mut Context<AgentChatView>,
    ) {
        match self {
            Self::One(entity) => {
                entity.update(cx, |input, cx| input.set_queued_submissions(previews, cx))
            }
            Self::Acp(entity) => {
                entity.update(cx, |input, cx| input.set_queued_submissions(previews, cx))
            }
        }
    }

    pub(super) fn set_pending_queue_blocked(&self, blocked: bool, cx: &mut Context<AgentChatView>) {
        match self {
            Self::One(entity) => {
                entity.update(cx, |input, cx| input.set_pending_queue_blocked(blocked, cx))
            }
            Self::Acp(entity) => {
                entity.update(cx, |input, cx| input.set_pending_queue_blocked(blocked, cx))
            }
        }
    }

    pub(super) fn focus_input(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::One(entity) => entity.update(cx, |input, cx| input.focus_input(window, cx)),
            Self::Acp(entity) => entity.update(cx, |input, cx| input.focus_input(window, cx)),
        }
    }

    pub(super) fn composer_text(&self, cx: &App) -> String {
        match self {
            Self::One(entity) => entity.read(cx).composer_text(cx),
            Self::Acp(entity) => entity.read(cx).composer_text(cx),
        }
    }

    pub(super) fn composer_attachments(&self, cx: &App) -> Vec<ImageAttachment> {
        match self {
            Self::One(entity) => entity.read(cx).composer_attachments().to_vec(),
            Self::Acp(entity) => entity.read(cx).composer_attachments().to_vec(),
        }
    }

    pub(super) fn set_composer_text(
        &self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<AgentChatView>,
    ) {
        match self {
            Self::One(entity) => {
                entity.update(cx, |input, cx| input.set_composer_text(text, window, cx))
            }
            Self::Acp(entity) => {
                entity.update(cx, |input, cx| input.set_composer_text(text, window, cx))
            }
        }
    }

    pub(super) fn set_composer_attachments(
        &self,
        images: Vec<ImageAttachment>,
        cx: &mut Context<AgentChatView>,
    ) {
        match self {
            Self::One(entity) => {
                entity.update(cx, |input, cx| input.set_composer_attachments(images, cx))
            }
            Self::Acp(entity) => {
                entity.update(cx, |input, cx| input.set_composer_attachments(images, cx))
            }
        }
    }

    pub(super) fn restore_to_composer(
        &self,
        text: &str,
        images: Vec<ImageAttachment>,
        window: &mut Window,
        cx: &mut Context<AgentChatView>,
    ) {
        match self {
            Self::One(entity) => entity.update(cx, |input, cx| {
                input.restore_to_composer(text, images, window, cx)
            }),
            Self::Acp(entity) => entity.update(cx, |input, cx| {
                input.restore_to_composer(text, images, window, cx)
            }),
        }
    }

    /// 正在放大预览的附件（图 + 名字）。覆盖层挂在面板根部，由这里取数据。
    pub(super) fn previewed_attachment(
        &self,
        cx: &App,
    ) -> Option<(std::sync::Arc<gpui::Image>, SharedString)> {
        let attachment = match self {
            Self::One(entity) => entity.read(cx).previewed_attachment().cloned(),
            Self::Acp(entity) => entity.read(cx).previewed_attachment().cloned(),
        }?;
        Some((attachment.image.clone(), attachment.name.clone().into()))
    }

    pub(super) fn close_attachment_preview(&self, cx: &mut App) {
        match self {
            Self::One(entity) => entity.update(cx, |input, cx| input.close_attachment_preview(cx)),
            Self::Acp(entity) => entity.update(cx, |input, cx| input.close_attachment_preview(cx)),
        }
    }
}

impl AgentChatView {
    /// 当前后端该用哪一份输入框。
    pub(super) fn composer(&self) -> ComposerSlot {
        self.composer_for(self.backend)
    }

    pub(super) fn composer_for(&self, backend: Backend) -> ComposerSlot {
        if backend == Backend::Acp {
            ComposerSlot::Acp(self.acp_input.clone())
        } else {
            ComposerSlot::One(self.input.clone())
        }
    }

    /// 没挂载的那份输入框。
    ///
    /// 面板级状态（运行旗标、排队）要往两份都写：只写当前挂的那份，另一份会带着
    /// 离开时的旧状态躺在内存里，下次挂上来就是错的样子。
    pub(super) fn other_composer(&self) -> ComposerSlot {
        if self.backend == Backend::Acp {
            ComposerSlot::One(self.input.clone())
        } else {
            ComposerSlot::Acp(self.acp_input.clone())
        }
    }

    /// 输入区当前那份输入框里的文字。
    pub(super) fn composer_text(&self, cx: &App) -> String {
        self.composer().composer_text(cx)
    }

    /// 两份输入框一起置「正在跑」旗标。
    ///
    /// 旗标描述的是**面板**是否在跑（本地跑一轮 / ACP 转场或跑一轮），不是某一份
    /// 组件自己的状态。只改当前挂载的那份，另一份会带着离开时的旧旗标躺在内存里，
    /// 下次挂上来就是错的按钮（该显示"发送"却是"停止"）。
    pub(super) fn set_composers_running(&mut self, running: bool, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_running(running, cx));
        self.acp_input
            .update(cx, |input, cx| input.set_running(running, cx));
    }

    /// 输入区当前那份输入框里挂着的附件。
    pub(super) fn composer_attachments(&self, cx: &App) -> Vec<ImageAttachment> {
        self.composer().composer_attachments(cx)
    }

    /// 输入区的渲染：按后端挂载 One agent 或 ACP 那份。
    pub(super) fn render_composer(&self) -> gpui::AnyElement {
        match self.composer() {
            ComposerSlot::One(entity) => entity.into_any_element(),
            ComposerSlot::Acp(entity) => entity.into_any_element(),
        }
    }

    /// 发现挂了另一份输入框时，把内容从旧那份搬到新那份。
    ///
    /// 两份组件各有自己的编辑器与附件，切换不会自动搬运：不搬的话，切到 ACP agent
    /// 后刚打的半句话就留在 One agent 那份里——用户看到的是空输入框，以为内容丢了。
    ///
    /// 判断放在渲染里（而不是每个赋值 `backend` 的地方）：漏一处就会出现
    /// 「切过去了但字留在另一份里」，而切换后必然渲染一帧。
    ///
    /// 会话切换排队的草稿优先：那是**目标会话自己**的内容，不该被"搬过去"的盖掉。
    pub(super) fn sync_composer_handover(&mut self, cx: &App) {
        if self.composer_backend == self.backend {
            return;
        }
        let previous = self.composer_backend;
        self.composer_backend = self.backend;
        if self.pending_input_draft.is_some() {
            return;
        }
        let from = self.composer_for(previous);
        let text = from.composer_text(cx);
        let images = from.composer_attachments(cx);
        self.pending_input_draft = Some(ComposerDraft { text, images });
    }
}
