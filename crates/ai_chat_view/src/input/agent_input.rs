//! 本地 One agent 的输入框:多行输入 + `@` 提及 + 图片附件 + 底部一行「模型 + 权限档位 + 发送」。
//!
//! 只装**本地运行时自己的东西**,而且只留两个控件:上层注入的**模型下拉**与**权限档位**
//!(即 ACP 那套工具执行模式)。计划 / 子代理 / 上下文资源 / 目标选择器 / Skill 入口 /
//! 工作区·分支·Worktree / 用量圆环都不在这里——那些要么归 ACP 那份输入框
//! ([`crate::input::AcpAgentInput`]),要么已经随本次精简撤掉。
//!
//! 布局:
//! - **附件条**:编辑器顶部的附件入口 + 图片缩略图(粘贴 / 附加);
//! - **编辑器**:基于 [`EditorState`] 的多行输入,注入 [`MentionCompletionProvider`] 实现 `@` 提及;
//! - **底部一行**:附件 / 模型 / 权限档位 / 发送·停止。
//!
//! 设计原则:输入框是"哑组件",只接收 [`AgentComposerContext`] 做展示并在交互时 emit
//! [`AgentInputEvent`];模型与执行模式都用上层注入的选项渲染内置下拉。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, EntityInputHandler, EventEmitter, ExternalPaths, FocusHandle,
    Focusable, InteractiveElement, IntoElement, Modifiers, ParentElement, PathPromptOptions,
    Pixels, Render, SharedString, StatefulInteractiveElement, StyleRefinement, Styled,
    Subscription, Window, div, img, prelude::FluentBuilder, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Editor, EditorState, InputEvent};
use gpui_component::popover::Popover;
use gpui_component::searchable_list::SearchableVec;
use gpui_component::select::{Select, SelectEvent, SelectGroup, SelectState};
use gpui_component::{ActiveTheme, Disableable, Icon, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;
use serde_json::Value;

use crate::input::PromptHistory;
use crate::input::attachment::ImageAttachment;
use crate::input::completion::ComposerCompletionProvider;
use crate::input::context::{AgentComposerContext, ComposerMenuOption, ComposerModelOption};
use crate::input::mention::{MentionCompletionProvider, MentionItem};
use crate::input::model_picker::{ModelChoice, model_groups, selected_model_index};
use crate::theme::{AgentChatTheme, sp};

/// AgentInput 对外事件。
#[derive(Clone, Debug)]
pub enum AgentInputEvent {
    /// 用户提交一条消息。
    Submit {
        /// 文本内容(含 `@提及` 原文)。
        text: String,
        /// 文本中被引用到的提及条目。
        mentions: Vec<MentionItem>,
        /// 附带的图片。
        images: Vec<ImageAttachment>,
    },
    /// 用户请求停止当前运行。
    Stop,
    /// 在顶部目标下拉中选择了目标。
    /// 将资源加入本会话资源池。
    /// 将资源移出本会话资源池。
    /// 选择资源池来源预设。
    /// 在内置下拉中选择了模型。
    SelectModel {
        id: SharedString,
        provider_id: SharedString,
        model: SharedString,
    },
    /// 在内置下拉中选择了工具执行模式。
    SelectExecutionMode { id: SharedString },
    /// 在底部上下文栏的工作区下拉中选择了工作区。
    SelectWorkspace { path: SharedString },
    /// 点击底部上下文栏工作区下拉里的「选择其它目录」，由宿主弹出目录选择器。
    BrowseWorkspace,
    /// 在底部上下文栏的分支下拉中选择了分支。
    SelectBranch { name: SharedString },
    /// 切换底部上下文栏的 Worktree 开关。
    ToggleWorktree { enabled: bool },
    /// 删除队列中第 `index` 条待执行提交。
    RemoveQueued { index: usize },
    /// 把队列中第 `index` 条拉回输入框继续编辑（由上层负责从队列移除）。
    EditQueued { index: usize },
    /// 回答 agent 的提问：`content` 已按属性名收集好，直接送回 ACP。
    SubmitElicitation { content: BTreeMap<String, Value> },
    /// 明确拒绝回答 agent 的提问。
    DeclineElicitation,
    /// 关掉这条提问（等价于取消，agent 会走降级路径）。
    CancelElicitation,
    /// 用户在用量详情里点了「查看历史」——由上层打开独立用量视图。
    OpenUsageHistory,
    /// 输入框内容（文字或附件）变了——上层据此防抖保存草稿。
    ///
    /// 附件变化不产生编辑器的 `InputEvent::Change`（图片不走文本编辑），
    /// 所以两头都要发：只挂在文字上，加完图直接关应用就会丢草稿。
    DraftChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryDirection {
    Previous,
    Next,
}

fn history_direction(key: &str, modifiers: &Modifiers) -> Option<HistoryDirection> {
    if modifiers.modified() {
        return None;
    }
    match key {
        "up" => Some(HistoryDirection::Previous),
        "down" => Some(HistoryDirection::Next),
        _ => None,
    }
}

fn cursor_is_at_history_boundary(direction: HistoryDirection, text: &str, cursor: usize) -> bool {
    match direction {
        HistoryDirection::Previous => cursor == 0,
        HistoryDirection::Next => cursor == text.len(),
    }
}

/// 当前会话中等待下一轮执行的提交摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedPromptPreview {
    pub text: SharedString,
    pub image_count: usize,
}

impl QueuedPromptPreview {
    pub fn new(text: impl Into<SharedString>, image_count: usize) -> Self {
        Self {
            text: text.into(),
            image_count,
        }
    }
}

const COMPOSER_EDITOR_MIN_ROWS: usize = 3;
const COMPOSER_EDITOR_MAX_ROWS: usize = 10;
const COMPOSER_EDITOR_FALLBACK_LINE_HEIGHT: f32 = 20.0;
const COMPOSER_EDITOR_VERTICAL_PADDING: f32 = 12.0;
/// 底部工具栏操作按钮(发送 / 排队 / 停止)的边长,同时也是工具栏控件高度。
const TOOLBAR_ACTION_BUTTON_SIZE: f32 = 32.0;
/// 底部那一行里权限档位 chip 的固定高度。
///
/// 与发送 / 附件按钮同高：整行只有一种控件高度，并排时不会参差。
const COMPOSER_CHIP_HEIGHT: f32 = TOOLBAR_ACTION_BUTTON_SIZE;
/// 权限档位 chip 收缩到的最小宽度（标签走省略号）。
///
/// 窄侧栏里模型与权限档位仍要在同一行：宽度不足靠 `flex_shrink` + `truncate` 消化，
/// 完整内容在点击后弹出的菜单里。
const COMPOSER_CHIP_MIN_WIDTH: f32 = 44.0;
/// 下拉触发器里除文字外的固定占位:左右内边距 + 箭头 + 间距。
const TRIGGER_CHROME_WIDTH: f32 = 48.0;
/// 模型触发器在这一行里能被压到的最小宽度。
///
/// 模型是这一行唯一 `flex_1` 的项:空间富余时它吸收余量,空间不足时缩到这里为止
/// (再缩下去连「9Router / deepsee…」都放不下,不如把宽度让给两侧的 chip)。
const MODEL_TRIGGER_MIN_WIDTH: f32 = 150.0;
/// 模型下拉弹层的宽度区间。
///
/// 组件库 `Select` 的 `menu_width` 默认是 `Length::Auto` —— 弹层宽度**取触发器宽度**
/// (见 `gpui_component::select` 的 `Length::Auto => this.w(bounds.size.width)`)。
/// 触发器被挤到 [`MODEL_TRIGGER_MIN_WIDTH`] 时标题只剩 `9Router / deepsee…`;
/// 弹层若跟着按 150px 渲染,用户点开看到的还是同一个省略号 ——「挤不下的点开看全文」
/// 这条兜底就失效了。所以这里按选项里最长的 `provider / model` 估宽度并夹在区间内:
/// 短列表不塌成一条,长列表收在上限,超宽时由组件库的 `Positioner` 按窗口边缘收敛
/// (它带 8px margin,会把浮层压回视口内)。
const MODEL_MENU_MIN_WIDTH: f32 = 320.0;
const MODEL_MENU_MAX_WIDTH: f32 = 400.0;
/// 弹层里除标签外还要留的宽度:选项行左右内边距 + 选中标记。
const MODEL_MENU_CHROME_WIDTH: f32 = 56.0;

fn composer_editor_height(state: &EditorState) -> Pixels {
    let rows = state
        .text()
        .to_string()
        .split('\n')
        .count()
        .clamp(COMPOSER_EDITOR_MIN_ROWS, COMPOSER_EDITOR_MAX_ROWS);
    let line_height = state
        .line_height()
        .unwrap_or(px(COMPOSER_EDITOR_FALLBACK_LINE_HEIGHT));
    line_height * rows as f32 + px(COMPOSER_EDITOR_VERTICAL_PADDING)
}

/// 底部那一行里单个 chip 的内容:`[图标] [可截断标签] [⌄]`。
///
/// 这个函数只管内容;承载它的容器(直接可点的 chip / 下拉触发器)由调用方决定,
/// 但都必须给足宽度约束(容器 `min_w_0` + 这里 `flex_1 min_w_0`)标签才会走省略号。
fn composer_chip_label(icon: IconName, label: SharedString) -> impl IntoElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_1()
        .child(Icon::new(icon).xsmall().flex_shrink_0())
        .child(div().flex_1().min_w_0().truncate().text_xs().child(label))
        .child(Icon::new(IconName::ChevronDown).xsmall().flex_shrink_0())
}

/// 模型下拉弹层的宽度:按最长的选项估,保证点开能读到完整的 `provider / model`。
///
/// 不能沿用组件库默认值(那是「触发器多宽弹层多宽」,见 [`MODEL_MENU_MIN_WIDTH`]),
/// 也不适合只给一个固定值:ACP agent 报上来的模型名长短差别很大,固定宽要么空一大片
/// 要么还是截断。这里按内容估(标签与副标题取宽者),再夹进区间。
fn model_menu_width(options: &[ComposerModelOption]) -> gpui::Rems {
    let longest = options
        .iter()
        .map(|option| {
            let label = estimated_label_width(&option.display_label());
            let hint = option
                .hint
                .as_deref()
                .map(estimated_label_width)
                .unwrap_or(0.0);
            label.max(hint)
        })
        .fold(0.0_f32, f32::max);
    sp((longest + MODEL_MENU_CHROME_WIDTH).clamp(MODEL_MENU_MIN_WIDTH, MODEL_MENU_MAX_WIDTH))
}

fn current_execution_mode_label(label: &SharedString) -> SharedString {
    if label.is_empty() {
        SharedString::from(t!("AgentUi.auto").to_string())
    } else {
        label.clone()
    }
}

/// 执行模式下拉触发器的宽度。
///
/// 执行模式只承载「自动 / 只读 / 手动确认」这类短标签,固定 124px 会在窄侧边栏里
/// 吃掉模型选择的空间,把发送 / 停止按钮挤成细条。这里按标签估算宽度并夹在区间内:
/// 中文短标签收窄,英文长标签仍保留原来的可用宽度上限。
fn execution_trigger_width(label: &str, queue_mode: bool) -> gpui::Rems {
    let (min, max) = if queue_mode {
        (72.0, 88.0)
    } else {
        (80.0, 124.0)
    };
    sp((estimated_label_width(label) + TRIGGER_CHROME_WIDTH).clamp(min, max))
}

/// 估算一段文案的渲染宽度:CJK 等全角字符按 13px,其余按 7.5px。
fn estimated_label_width(label: &str) -> f32 {
    label
        .chars()
        .map(|ch| if is_full_width_char(ch) { 13.0 } else { 7.5 })
        .sum()
}

/// 常见全角字符区间(中日韩文字、全角标点等)。
fn is_full_width_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x20000..=0x3FFFD
    )
}

/// Agent 输入框组件。
pub struct AgentInput {
    focus_handle: FocusHandle,
    input_state: Entity<EditorState>,
    /// 可被 `@` 引用的条目(同时用于补全 provider 与提交时解析)。
    mentions: Arc<Vec<MentionItem>>,
    /// 当前图片附件。
    attachments: Vec<ImageAttachment>,
    /// 正在放大预览的附件 id。
    ///
    /// 只存 id 不存图：附件可能在预览期间被移除，存 id 时下一帧自然读不到，
    /// 覆盖层跟着消失，不会留下一张已经不在列表里的「幽灵图」。
    attachment_preview: Option<String>,
    /// 已提交过的文本历史，用于在输入框中通过上下键浏览。
    history: PromptHistory,
    /// 当前会话等待下一轮执行的提交摘要。
    queued_submissions: Vec<QueuedPromptPreview>,
    /// 队列因 ACP 连接或会话过渡失败而暂停，可由用户显式停止并清空。
    pending_queue_blocked: bool,
    /// 是否正在运行(运行中显示「停止」)。
    is_running: bool,
    /// 上层注入的展示上下文（模型与权限档位的当前文案）。
    context: AgentComposerContext,
    /// 模型下拉选项(上层注入)。
    model_options: Vec<ComposerModelOption>,
    /// 模型下拉的组件库状态。
    ///
    /// 选项的注入点只有 `Context`、拿不到窗口，而 `SelectState` 的更新需要窗口；
    /// 所以注入时只置脏标记，真正的同步在 `render`(持有窗口)时做。
    model_select: Entity<SelectState<SearchableVec<SelectGroup<ModelChoice>>>>,
    /// 见 `model_select`:注入新选项后置位,`render` 消费后清掉。
    model_select_dirty: bool,
    /// 权限档位下拉的选项(上层注入)。
    execution_mode_options: Vec<ComposerMenuOption>,
    /// 权限档位下拉的开合(受控)。
    mode_menu_open: bool,
    /// 可选的局部聊天主题。终端侧边栏会注入终端主题,普通 Agent tab 继续使用应用主题。
    theme: Option<AgentChatTheme>,
    edge_to_edge: bool,
    _subscriptions: Vec<Subscription>,
}

impl AgentInput {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_mentions(
            Vec::new(),
            t!("AgentUi.input_placeholder").to_string(),
            window,
            cx,
        )
    }

    pub fn with_mentions(
        mentions: Vec<MentionItem>,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mentions = Arc::new(mentions);
        let placeholder = placeholder.into();
        let provider_items = (*mentions).clone();
        let input_state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .line_number(false)
                .soft_wrap(true)
                .submit_on_enter(true)
                .placeholder(placeholder);
            state.lsp_mut().completion_provider = Some(Rc::new(
                ComposerCompletionProvider::mentions(provider_items),
            ));
            state
        });

        let enter_sub = cx.subscribe_in(&input_state, window, |this, _state, event, window, cx| {
            match event {
                InputEvent::PressEnter {
                    secondary, shift, ..
                } if !secondary && !shift => {
                    this.submit(window, cx);
                }
                // 文字变了就转发一条：草稿要保存（防抖在上层做，这里只说事实）。
                InputEvent::Change => {
                    cx.emit(AgentInputEvent::DraftChanged);
                }
                _ => {}
            }
        });

        // 编辑器聚焦时,cmd/ctrl-v 会被 InputState 的 Paste action 先消费,外层
        // capture_key_down 不会触发(action 在 key_down 监听之前分发)。keystroke
        // 拦截器在 action 之前运行,用它在聚焦时把剪贴板图片补进附件(文本粘贴仍交给
        // InputState,二者互不影响)。
        let weak = cx.entity().downgrade();
        let paste_sub = cx.intercept_keystrokes(move |ev, window, cx| {
            if ev.keystroke.key != "v" || !ev.keystroke.modifiers.secondary() {
                return;
            }
            let Some(this) = weak.upgrade() else {
                return;
            };
            let input_state = this.read(cx).input_state.clone();
            if !input_state.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            let atts = ImageAttachment::from_clipboard(cx);
            if atts.is_empty() {
                return;
            }
            this.update(cx, |this, cx| this.add_attachments(atts, cx));
        });

        let weak = cx.entity().downgrade();
        let history_sub = cx.intercept_keystrokes(move |ev, window, cx| {
            let Some(direction) =
                history_direction(ev.keystroke.key.as_ref(), &ev.keystroke.modifiers)
            else {
                return;
            };
            let Some(this) = weak.upgrade() else {
                return;
            };
            let input_state = this.read(cx).input_state.clone();
            let can_navigate = input_state.update(cx, |state, cx| {
                let text = state.text().to_string();
                state.focus_handle(cx).is_focused(window)
                    && state.selected_range().is_empty()
                    && state.marked_text_range(window, cx).is_none()
                    && MentionCompletionProvider::extract_mention_query(&text, state.cursor())
                        .is_none()
                    && cursor_is_at_history_boundary(direction, &text, state.cursor())
            });
            if !can_navigate {
                return;
            }

            let handled = this.update(cx, |this, cx| this.navigate_history(direction, window, cx));
            if handled {
                cx.stop_propagation();
            }
        });

        // 模型下拉交给组件库的 `Select`(自带搜索框、虚拟滚动与键盘导航)。
        // 选项这里先留空:它由 `set_menu_options` 置脏、`render` 里带窗口灌进去。
        let model_select: Entity<SelectState<SearchableVec<SelectGroup<ModelChoice>>>> =
            cx.new(|cx| {
                SelectState::<SearchableVec<SelectGroup<ModelChoice>>>::new(
                    SearchableVec::new(Vec::new()),
                    None,
                    window,
                    cx,
                )
                .searchable(true)
            });
        let model_select_sub = cx.subscribe(
            &model_select,
            |_this: &mut Self,
             _state: Entity<SelectState<SearchableVec<SelectGroup<ModelChoice>>>>,
             event: &SelectEvent<SearchableVec<SelectGroup<ModelChoice>>>,
             cx: &mut Context<Self>| {
                // 用户确认了一项:把完整选项回报给上层(它据此重建运行时)。
                let SelectEvent::Confirm(Some(option)) = event else {
                    return;
                };
                cx.emit(AgentInputEvent::SelectModel {
                    id: option.id.clone(),
                    provider_id: option.provider_id.clone(),
                    model: option.model.clone(),
                });
            },
        );

        Self {
            focus_handle: cx.focus_handle(),
            input_state,
            mentions,
            attachments: Vec::new(),
            attachment_preview: None,
            history: PromptHistory::default(),
            queued_submissions: Vec::new(),
            pending_queue_blocked: false,
            is_running: false,
            context: AgentComposerContext::default(),
            model_options: Vec::new(),
            model_select,
            model_select_dirty: false,
            execution_mode_options: Vec::new(),
            mode_menu_open: false,
            theme: None,
            edge_to_edge: false,
            _subscriptions: vec![enter_sub, paste_sub, history_sub, model_select_sub],
        }
    }

    pub fn set_theme(&mut self, theme: Option<AgentChatTheme>, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn local_theme(&self, cx: &App) -> AgentChatTheme {
        self.theme
            .clone()
            .unwrap_or_else(|| AgentChatTheme::from_app(cx))
    }

    pub fn set_edge_to_edge(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.edge_to_edge = enabled;
        cx.notify();
    }

    /// 当前可 `@` 引用的条目标签。
    ///
    /// 只为测试开门：补全菜单的内容由 provider 在后台算，测试直接从源头断言
    /// 「工作区文件到底有没有并进来」比在菜单渲染层翻找稳。
    #[cfg(test)]
    pub(crate) fn mention_labels(&self) -> Vec<String> {
        self.mentions
            .iter()
            .map(|mention| mention.label.clone())
            .collect()
    }

    /// 更新可引用的提及条目(同时刷新补全 provider)。
    pub fn set_mentions(&mut self, mentions: Vec<MentionItem>, cx: &mut Context<Self>) {
        self.mentions = Arc::new(mentions);
        self.refresh_completion_provider(cx);
    }

    fn refresh_completion_provider(&self, cx: &mut Context<Self>) {
        let mentions = (*self.mentions).clone();
        self.input_state.update(cx, |state, _| {
            state.lsp_mut().completion_provider =
                Some(Rc::new(ComposerCompletionProvider::mentions(mentions)));
        });
    }

    /// 注入展示上下文(顶部 Context Bar + 底部模式/模型当前文案)。
    pub fn set_context(&mut self, context: AgentComposerContext, cx: &mut Context<Self>) {
        self.context = context;
        cx.notify();
    }

    /// 注入底部模型与工具执行模式下拉的选项。
    pub fn set_menu_options(
        &mut self,
        model_options: Vec<ComposerModelOption>,
        execution_mode_options: Vec<ComposerMenuOption>,
        cx: &mut Context<Self>,
    ) {
        if self.model_options != model_options {
            self.model_options = model_options;
            // `SelectState` 的更新要窗口,而这里只有 `Context`;置脏,`render` 时同步。
            self.model_select_dirty = true;
        }
        self.execution_mode_options = execution_mode_options;
        cx.notify();
    }

    /// 运行旗标（测试断言用）。
    #[cfg(test)]
    pub(crate) fn is_running(&self) -> bool {
        self.is_running
    }

    /// 设置运行状态(决定显示「发送」还是「停止」)。
    pub fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        if self.is_running != running {
            self.is_running = running;
            if running {
                self.mode_menu_open = false;
            }
            cx.notify();
        }
    }

    /// 更新当前会话等待下一轮执行的提交摘要。
    pub fn set_queued_submissions(
        &mut self,
        submissions: Vec<QueuedPromptPreview>,
        cx: &mut Context<Self>,
    ) {
        if self.queued_submissions != submissions {
            self.queued_submissions = submissions;
            cx.notify();
        }
    }

    /// 设置当前会话的待执行队列是否处于阻塞状态。
    pub fn set_pending_queue_blocked(&mut self, blocked: bool, cx: &mut Context<Self>) {
        if self.pending_queue_blocked != blocked {
            self.pending_queue_blocked = blocked;
            cx.notify();
        }
    }

    /// 聚焦输入框。
    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        let handle = self.input_state.read(cx).focus_handle(cx);
        handle.focus(window, cx);
    }

    /// 当前输入框文本。
    pub fn composer_text(&self, cx: &App) -> String {
        self.input_state.read(cx).value().to_string()
    }

    /// 把一段排队提交放回输入框并聚焦（供「编辑队列项」使用）。
    ///
    /// 文本与附件一起回填，避免「编辑」变成静默丢附件。
    /// **从队列中移除由上层完成**：队列事实源在 `AgentChatView::pending_submissions`，
    /// 输入框只是渲染副本。
    pub fn restore_to_composer(
        &mut self,
        text: &str,
        images: Vec<ImageAttachment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input_state.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        if !images.is_empty() {
            self.add_attachments(images, cx);
        }
        self.focus_input(window, cx);
        cx.notify();
    }

    /// 设置输入框文本，不动附件、不抢焦点（会话草稿恢复专用）。
    ///
    /// 与 [`Self::restore_to_composer`] 的区别：那是「用户显式编辑」——要聚焦、
    /// 要带回附件；这里是「切了个会话，输入框该显示那个会话自己的草稿」——
    /// 只是换内容，用户正在别处打字的话不该被拽走焦点。
    pub fn set_composer_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input_state.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        // `set_value` 是程序化写入，不产生编辑器的 `InputEvent::Change`，
        // 但内容确实变了（会话切换恢复草稿 / 起手话填词），草稿该跟着走。
        cx.emit(AgentInputEvent::DraftChanged);
        cx.notify();
    }

    /// 当前输入框挂着的图片附件（会话草稿捕获用，只读）。
    pub fn composer_attachments(&self) -> &[ImageAttachment] {
        &self.attachments
    }

    /// 正在放大预览的附件（没有则 `None`）。
    ///
    /// 覆盖层画在上层面板根部，不在输入框自己的树里：输入框只占面板底部一条，
    /// 绝对定位的子元素跳不出它的尺寸。
    pub fn previewed_attachment(&self) -> Option<&ImageAttachment> {
        let id = self.attachment_preview.as_deref()?;
        self.attachments.iter().find(|att| att.id == id)
    }

    /// 打开/关闭某张附件的预览（再点一下同一张就是关）。
    pub fn toggle_attachment_preview(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.attachments.iter().any(|att| att.id == id) {
            return;
        }
        self.attachment_preview = if self.attachment_preview.as_deref() == Some(id) {
            None
        } else {
            Some(id.to_string())
        };
        cx.notify();
    }

    /// 关掉附件预览（点背板 / 点关闭按钮；本来就是关的时候是 no-op）。
    pub fn close_attachment_preview(&mut self, cx: &mut Context<Self>) {
        if self.attachment_preview.take().is_some() {
            cx.notify();
        }
    }

    /// 整体替换输入框的图片附件（会话草稿恢复专用，与 [`Self::set_composer_text`] 同一套纪律）。
    ///
    /// **整体替换**是防串台的关键：切到一个没有附件草稿的会话时，上一会话挂在
    /// 输入框里的图片必须被清掉，而不是留下来跟着新会话一起发出去。
    /// 不聚焦、不追加：只是换内容。
    pub fn set_composer_attachments(
        &mut self,
        images: Vec<ImageAttachment>,
        cx: &mut Context<Self>,
    ) {
        self.attachments = images;
        cx.emit(AgentInputEvent::DraftChanged);
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input_state.read(cx).value().to_string();
        if text.trim().is_empty() && self.attachments.is_empty() {
            return;
        }

        self.history.record(&text);
        let mentions = self.referenced_mentions(&text);
        let images = std::mem::take(&mut self.attachments);

        cx.emit(AgentInputEvent::Submit {
            text,
            mentions,
            images,
        });

        self.input_state.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        cx.notify();
    }

    fn navigate_history(
        &mut self,
        direction: HistoryDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self.input_state.read(cx).text().to_string();
        let was_browsing = self.history.is_browsing();
        let replacement = match direction {
            HistoryDirection::Previous => self.history.previous(&current),
            HistoryDirection::Next => self.history.next(),
        };
        let Some(replacement) = replacement else {
            return was_browsing;
        };

        self.input_state.update(cx, |state, cx| {
            state.replace_all(replacement, window, cx);
            let new_len = state.text().len();
            state.set_selected_range(new_len..new_len, cx);
        });
        true
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        cx.emit(AgentInputEvent::Stop);
    }

    /// 文本中实际引用到的提及条目(其 `@label` 出现在文本里)。
    fn referenced_mentions(&self, text: &str) -> Vec<MentionItem> {
        referenced_mentions_in_text(text, self.mentions.as_ref())
    }

    /// 从 Finder 拖进来的文件。
    ///
    /// 图片落成附件;其余(文本 / PDF / 目录)把路径插到光标处——附件最终是作为
    /// 多模态图片发给模型的,塞一个 PDF 进去只会在提交时报错,而路径写进正文
    /// 更有用:agent 自己会去读。两种情况都算内容变化,防抖落草稿由上层接。
    fn add_dropped_paths(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let mut attachments = Vec::new();
        let mut texts = Vec::new();
        for path in paths {
            match ImageAttachment::from_path(path) {
                Some(att) => attachments.push(att),
                None => texts.push(format!("{} ", path.display())),
            }
        }
        let staged = !attachments.is_empty() || !texts.is_empty();
        self.add_attachments(attachments, cx);
        for text in texts {
            self.input_state
                .update(cx, |state, cx| state.insert(text, window, cx));
        }
        if staged {
            self.focus_input(window, cx);
        }
    }

    fn add_attachments(&mut self, mut atts: Vec<ImageAttachment>, cx: &mut Context<Self>) {
        if atts.is_empty() {
            return;
        }
        self.attachments.append(&mut atts);
        cx.emit(AgentInputEvent::DraftChanged);
        cx.notify();
    }

    /// 打开系统文件对话框选择图片。
    fn open_file_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("AgentUi.choose_images").to_string().into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let atts: Vec<ImageAttachment> = paths
                .iter()
                .filter_map(|p| ImageAttachment::from_path(p))
                .collect();
            let _ = this.update(cx, |this, cx| this.add_attachments(atts, cx));
        })
        .detach();
    }

    fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.attachments.retain(|a| a.id != id);
        cx.emit(AgentInputEvent::DraftChanged);
        // 被移除的那张正好在预览里：连同覆盖层一起关掉。
        if self.attachment_preview.as_deref() == Some(id) {
            self.attachment_preview = None;
        }
        cx.notify();
    }

    /// 底部那一行的「权限级别」下拉。
    ///
    /// 承载的仍是 ACP 工具执行模式(自动 / 只读 / 手动确认):状态、选项与回调都与
    /// 原先工具栏里的那个下拉一致,只是收成了 chip 形态。
    fn render_permission_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.mode_menu_open;
        let label = current_execution_mode_label(&self.context.execution_mode_label);
        let data = ModeContentData {
            execution_mode_label: label.clone(),
            options: self.execution_mode_options.clone(),
        };

        let theme = self.local_theme(cx);
        let queue_mode = self.is_running || self.pending_queue_blocked;
        let chip_width = execution_trigger_width(&label, queue_mode);
        let trigger = themed_outline_button(
            Button::new("agent-permission")
                .debug_selector(|| "agent-input-permission".to_string())
                .small()
                .w_full()
                .h(sp(COMPOSER_CHIP_HEIGHT))
                .justify_between()
                .outline()
                .disabled(self.is_running)
                .child(composer_chip_label(IconName::Key, label)),
            &theme,
        );

        div()
            .w(chip_width)
            .min_w(sp(COMPOSER_CHIP_MIN_WIDTH))
            .flex_shrink(1.0)
            .h(sp(COMPOSER_CHIP_HEIGHT))
            .overflow_hidden()
            .child(
                Popover::new("agent-permission-popover")
                    .p_0()
                    .open(is_open)
                    .trigger_style(StyleRefinement::default().w_full())
                    .on_open_change({
                        let view = view.clone();
                        move |open, _window, cx| {
                            let open = *open;
                            view.update(cx, |this, cx| {
                                this.mode_menu_open = open && !this.is_running;
                                cx.notify();
                            });
                        }
                    })
                    .trigger(trigger)
                    .content({
                        let view = view.clone();
                        let theme = theme.clone();
                        move |_state, _window, cx| {
                            render_mode_content(view.clone(), data.clone(), &theme, cx)
                        }
                    }),
            )
    }

    /// 底部那一行里「附件 / 模型与权限 / 操作按钮」三段中的中间段。
    ///
    /// 本地 agent 的下拉只有两个：**模型**与**权限档位**。工作区 / 分支 / Worktree /
    /// 目标选择器、用量圆环都不在这里——那些是 ACP 那份输入框的事（见
    /// [`crate::input::AcpAgentInput`]）。
    ///
    /// 这一组独占剩余宽度：空间不足时先由本组自己的 `overflow_hidden` 兜底，
    /// 绝不把右侧的发送按钮挤出可视区。
    fn render_context_group(
        &self,
        cx: &mut Context<Self>,
        row_gap: gpui::Rems,
    ) -> impl IntoElement + use<> {
        let model_label = match &self.context.model {
            Some(m) if m.model_only => m.model.clone(),
            Some(m) => SharedString::from(format!("{} / {}", m.provider, m.model)),
            None => SharedString::from(t!("AgentUi.select_model").to_string()),
        };
        let model_min_width = if self.is_running || self.pending_queue_blocked {
            96.0
        } else {
            MODEL_TRIGGER_MIN_WIDTH
        };
        // 触发器会被挤扁,弹层不能跟着缩 —— 见 `model_menu_width`。
        let model_menu_width = model_menu_width(&self.model_options);
        let action_button_size = sp(TOOLBAR_ACTION_BUTTON_SIZE);

        // 模型用 `flex_1` 吸收整行余量:它是唯一需要「搜索 + 滚动 + 键盘导航」的控件,
        // 权限 chip 保持内容宽度。
        h_flex()
            .debug_selector(|| "agent-input-context-group".to_string())
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap(row_gap)
            .text_color(self.local_theme(cx).foreground)
            .child(
                div()
                    .flex_1()
                    .min_w(sp(model_min_width))
                    .h(action_button_size)
                    .overflow_hidden()
                    .child(self.render_model_menu(model_label, model_menu_width))
                    .debug_selector(|| "agent-input-model-control".to_string()),
            )
            .child(self.render_permission_menu(cx))
    }

    /// 模型下拉:交给组件库的 `Select` —— 搜索框、滚动、键盘导航都由它提供。
    ///
    /// 选项不从这里传:它们在 `set_menu_options` 时置脏、`render` 里灌进 `model_select`。
    ///
    /// `menu_width` 必须显式传:组件库默认让弹层跟着触发器宽度走,而这一行的触发器会被
    /// 挤到 [`MODEL_TRIGGER_MIN_WIDTH`],弹层跟着缩就等于「点开也看不全」—— 那条
    /// 「挤不下的点开看全文」的兜底会在最需要它的宽度上失效(见 [`model_menu_width`])。
    fn render_model_menu(
        &self,
        placeholder: SharedString,
        menu_width: gpui::Rems,
    ) -> impl IntoElement + use<> {
        Select::new(&self.model_select)
            .id("agent-model")
            .w_full()
            .h(sp(32.0))
            .small()
            .placeholder(placeholder)
            .search_placeholder(t!("AgentUi.search_model").to_string())
            .menu_width(menu_width)
            .menu_max_h(sp(320.0))
            .disabled(self.is_running)
    }

    /// 把当前选项与选中项灌进组件库的 `Select`。
    ///
    /// 只在 `model_select_dirty` 时调用:`set_items` 会触发重渲染,必须靠脏标记保证
    /// 「一次同步即收敛」,否则渲染 → 通知 → 渲染会自己转起来。
    fn sync_model_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let groups = model_groups(&self.model_options);
        let selected = selected_model_index(&self.model_options, self.context.model.as_ref());
        self.model_select.update(cx, |state, cx| {
            state.set_items(groups, window, cx);
            state.set_selected_index(selected, window, cx);
        });
    }

    fn render_attachments(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let theme = self.local_theme(cx);
        let thumbs: Vec<gpui::AnyElement> = self
            .attachments
            .iter()
            .map(|att| {
                let preview_id = att.id.clone();
                let remove_id = att.id.clone();
                let element_id = att.id.clone();
                let previewing = self.attachment_preview.as_deref() == Some(att.id.as_str());
                div()
                    .id(SharedString::from(format!("agent-attachment-{element_id}")))
                    .relative()
                    .debug_selector(move || format!("agent-attachment-{element_id}"))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_attachment_preview(&preview_id, cx);
                    }))
                    .child(
                        img(att.image.clone())
                            .w(sp(56.0))
                            .h(sp(56.0))
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(if previewing {
                                theme.accent
                            } else {
                                theme.border
                            }),
                    )
                    .child(
                        div().absolute().top_0().right_0().child(
                            Button::new(SharedString::from(format!("rm-att-{remove_id}")))
                                .icon(IconName::Close)
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_attachment(&remove_id, cx);
                                })),
                        ),
                    )
                    .into_any_element()
            })
            .collect();

        Some(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap_2()
                .px_3()
                .pt_2()
                .children(thumbs)
                .into_any_element(),
        )
    }

    fn render_queued_submissions(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.queued_submissions.is_empty() {
            return None;
        }

        let theme = self.local_theme(cx);
        let danger = cx.theme().danger;
        let count = self.queued_submissions.len();
        let mut items = v_flex().w_full().gap_1();
        for (index, submission) in self.queued_submissions.iter().enumerate() {
            let text = if submission.text.trim().is_empty() {
                t!("AgentUi.attachment_count", count = submission.image_count).to_string()
            } else if submission.image_count == 0 {
                submission.text.to_string()
            } else {
                format!(
                    "{} · {}",
                    submission.text,
                    t!("AgentUi.attachment_count", count = submission.image_count)
                )
            };
            items = items.child(
                h_flex()
                    .debug_selector(|| "agent-input-queued-item".to_string())
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .bg(theme.panel)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{}", index + 1)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(text))
                    .child(
                        Button::new(SharedString::from(format!("agent-queued-edit-{index}")))
                            .icon(IconName::Edit)
                            .ghost()
                            .small()
                            .tooltip(t!("AgentUi.edit_queued_item").to_string())
                            .on_click(cx.listener(move |_this, _, window, cx| {
                                cx.emit(AgentInputEvent::EditQueued { index });
                                // 焦点在此刻转移由上层回填文本后再落定；
                                // 这里只负责把窗口从按钮上摘下来。
                                window.refresh();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("agent-queued-remove-{index}")))
                            .icon(IconName::Delete)
                            .ghost()
                            .small()
                            .text_color(danger)
                            .tooltip(t!("AgentUi.remove_queued_item").to_string())
                            .on_click(cx.listener(move |_this, _, _window, cx| {
                                cx.emit(AgentInputEvent::RemoveQueued { index });
                                cx.notify();
                            })),
                    ),
            );
        }

        Some(
            v_flex()
                .debug_selector(|| "agent-input-queued".to_string())
                .w_full()
                .min_w_0()
                .gap_1()
                .px_3()
                .pt_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.foreground)
                        .child(t!("AgentUi.queued_for_next_turn", count = count).to_string()),
                )
                .child(items)
                .into_any_element(),
        )
    }

    fn render_editor_top_bar(&self) -> impl IntoElement + use<> {
        // 附件入口在底部 chips 行（render_toolbar）；顶栏只剩撤销，靠右排布。
        h_flex()
            .w_full()
            .items_center()
            .justify_end()
            .px_3()
            .pt_2()
            .pb_1()
            .child(
                Button::new("agent-editor-undo")
                    .icon(IconName::Undo)
                    .ghost()
                    .small()
                    .tooltip(t!("AgentUi.undo").to_string()),
            )
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let running = self.is_running;
        let queue_mode = running || self.pending_queue_blocked;
        let row_gap = if queue_mode { sp(4.0) } else { sp(8.0) };
        let action_button_size = sp(TOOLBAR_ACTION_BUTTON_SIZE);
        let attach_count = self.attachments.len();

        // 左段:附件入口 + 附件计数(固定宽度,不参与收缩)。
        let attach_group = h_flex()
            .flex_shrink_0()
            .items_center()
            .gap(row_gap)
            .child(
                Button::new("agent-attach")
                    .icon(IconName::File)
                    .ghost()
                    .small()
                    .tooltip(t!("AgentUi.attach_images").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.open_file_picker(window, cx))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("AgentUi.attachment_count", count = attach_count).to_string()),
            )
            .child(div().h(sp(18.0)).w(px(1.0)).bg(theme.border));

        // 中段:模型 + 权限档位。
        let context_group = self.render_context_group(cx, row_gap);

        let mut toolbar = h_flex()
            .debug_selector(|| "agent-input-toolbar".to_string())
            .w_full()
            .min_w_0()
            .items_center()
            .text_color(theme.foreground)
            .gap(row_gap)
            .px_3()
            .py_2()
            // 极端窄宽度下宁可裁掉中段的尾巴,也不让内容画到输入卡片外面。
            .overflow_hidden()
            .flex_shrink_0()
            .child(attach_group)
            .child(context_group);

        if queue_mode {
            toolbar = toolbar
                .child(
                    div()
                        .w(action_button_size)
                        .h(action_button_size)
                        .flex_shrink_0()
                        .debug_selector(|| "agent-input-queue-send".to_string())
                        .child(
                            Button::new("agent-queue-send")
                                .debug_selector(|| "agent-queue-send-button".to_string())
                                .icon(IconName::ArrowUp)
                                .primary()
                                .small()
                                .size(action_button_size)
                                .tooltip(t!("AgentUi.queue_for_next_turn").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .w(action_button_size)
                        .h(action_button_size)
                        .flex_shrink_0()
                        .debug_selector(|| "agent-input-stop".to_string())
                        .child(
                            Button::new("agent-stop")
                                .debug_selector(|| "agent-stop-button".to_string())
                                .icon(IconName::Close)
                                .danger()
                                .small()
                                .size(action_button_size)
                                .tooltip(t!("AgentUi.stop").to_string())
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        ),
                );
        } else {
            toolbar = toolbar.child(
                div()
                    .w(action_button_size)
                    .h(action_button_size)
                    .flex_shrink_0()
                    .debug_selector(|| "agent-input-send-control".to_string())
                    .child(
                        Button::new("agent-send")
                            .debug_selector(|| "agent-send-button".to_string())
                            .icon(IconName::ArrowUp)
                            .primary()
                            .small()
                            .size(action_button_size)
                            .tooltip(t!("AgentUi.send").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            );
        }

        toolbar
    }
}

fn referenced_mentions_in_text(text: &str, mentions: &[MentionItem]) -> Vec<MentionItem> {
    mentions
        .iter()
        .filter(|mention| text_contains_mention(text, &mention.label))
        .cloned()
        .collect()
}

fn themed_outline_button(button: Button, theme: &AgentChatTheme) -> Button {
    button
        .bg(theme.panel)
        .border_color(theme.border)
        .text_color(theme.foreground)
}

fn text_contains_mention(text: &str, label: &str) -> bool {
    mention_match_end(text, &format!("@{label}"))
        || text.contains(&format!("@`{label}`"))
        || text.contains(&format!("@\"{label}\""))
}

fn mention_match_end(text: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(offset) = text[start..].find(needle) {
        let end = start + offset + needle.len();
        if text[end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_mention_name_char(ch))
        {
            return true;
        }
        start = end;
    }
    false
}

fn is_mention_name_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-')
}

impl EventEmitter<AgentInputEvent> for AgentInput {}

#[derive(Clone)]
struct ModeContentData {
    execution_mode_label: SharedString,
    options: Vec<ComposerMenuOption>,
}

struct ModeOptionRow {
    option: ComposerMenuOption,
    selected: bool,
}

fn context_group_label(label: impl Into<SharedString>, theme: &AgentChatTheme) -> gpui::AnyElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label.into())
        .into_any_element()
}

fn render_mode_content(
    view: Entity<AgentInput>,
    data: ModeContentData,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let mut col = v_flex()
        .p_1()
        .gap(sp(2.0))
        .min_w(sp(320.0))
        .bg(theme.background)
        .text_color(theme.foreground);

    col = col.child(context_group_label(t!("AgentUi.mode").to_string(), theme));
    for option in data.options {
        let selected = option.label == data.execution_mode_label;
        col = col.child(mode_option_row(
            view.clone(),
            ModeOptionRow { option, selected },
            theme,
            cx,
        ));
    }

    col.into_any_element()
}

fn mode_option_row(
    view: Entity<AgentInput>,
    row: ModeOptionRow,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();
    let selected_fg = theme.accent;
    let id = row.option.id.clone();
    let row_id = SharedString::from(format!("agent-execution-mode-opt-{id}"));
    let mut inner = v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(1.0))
        .child(div().text_sm().truncate().child(row.option.label));
    if let Some(hint) = row.option.hint {
        inner = inner.child(div().text_xs().text_color(muted).child(hint));
    }

    h_flex()
        .id(row_id)
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(cx.theme().radius)
        .cursor_pointer()
        .when(row.selected, |this| {
            this.bg(selected_bg).text_color(theme.foreground)
        })
        .hover(move |this| this.bg(hover_bg))
        .child(inner)
        .when(row.selected, |this| {
            this.child(Icon::new(IconName::Check).xsmall().text_color(selected_fg))
        })
        .on_click(move |_, _window, cx| {
            let id = id.clone();
            view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                this.mode_menu_open = false;
                cx.emit(AgentInputEvent::SelectExecutionMode { id });
                cx.notify();
            });
        })
        .into_any_element()
}

/// 工作区下拉的菜单内容。
impl Focusable for AgentInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 模型下拉换了选项 / 选中项时,趁手里有窗口把最新的灌进组件库的 state。
        if self.model_select_dirty {
            self.sync_model_select(_window, cx);
            self.model_select_dirty = false;
        }
        let attachments = self.render_attachments(cx);
        let editor_top_bar = self.render_editor_top_bar();
        let queued_submissions = self.render_queued_submissions(cx);
        let toolbar = self.render_toolbar(cx);
        let theme = self.local_theme(cx);
        let input_state = self.input_state.read(cx);
        let input_focused = input_state.focus_handle(cx).is_focused(_window);
        let editor_height = composer_editor_height(input_state);
        // 拖放悬停时的染色。`drag_over` 的闭包要 `'static`,所以先把颜色取成
        // 本地副本(Hsla 是 Copy),而不是把 `theme` 移进去——后面还有别的闭包要用。
        let drop_wash = theme.accent.opacity(0.08);
        let drop_ring = theme.accent;

        // 输入卡片：带边框的那一块（模型、权限与附件、发送按钮同处卡片底部的**同一行**）。
        let card = v_flex()
            .debug_selector(|| "agent-input-card".to_string())
            .w_full()
            .min_w_0()
            .bg(theme.background)
            .text_color(theme.foreground)
            .when(!self.edge_to_edge, |this| {
                this.rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .shadow_sm()
            })
            .when(self.edge_to_edge, |this| this.min_h_0())
            // 从 Finder 拖文件进来:悬停给卡片染一层,松手落成附件或路径。
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.bg(drop_wash).border_color(drop_ring)
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.add_dropped_paths(paths.paths(), window, cx);
            }))
            // 附件预览（如果有）
            .children(attachments)
            .child(editor_top_bar)
            .when_some(queued_submissions, |this, queued| this.child(queued))
            // 中部：多行输入框
            .child(
                div()
                    .debug_selector(|| "agent-input-editor".to_string())
                    .w_full()
                    .min_w_0()
                    .px_3()
                    .pt_1()
                    .max_h(sp(220.0))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .border_1()
                            .rounded(cx.theme().radius)
                            .border_color(if input_focused {
                                theme.accent
                            } else {
                                theme.border
                            })
                            .bg(theme.background)
                            .child(
                                Editor::new(&self.input_state)
                                    .w_full()
                                    .h(editor_height)
                                    .appearance(false)
                                    .bordered(false)
                                    .text_color(theme.foreground),
                            ),
                    ),
            )
            // 底部：附件 / 模型 / 权限 / 发送，一整行
            .child(toolbar);

        v_flex()
            .id("agent-input-root")
            .debug_selector(|| "agent-input-root".to_string())
            .track_focus(&self.focus_handle)
            .w_full()
            .min_w_0()
            .when(self.edge_to_edge, |this| this.min_h_0().flex_shrink_0())
            .when(!self.edge_to_edge, |this| this.flex_shrink_0())
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::context::ComposerModel;
    use gpui::{Modifiers, Pixels, TestAppContext, VisualTestContext, size};

    /// 底栏（附件、模型、权限、发送）全部放下的最小宽度。
    ///
    /// 底栏固定是 `[附件][计数][模型][权限][发送]`：附件组与发送按钮不参与收缩，
    /// 模型与权限各自还有可读性下限；再窄就轮到中段的 `overflow_hidden` 裁尾巴了
    /// （发送按钮始终保留）。布局类测试一律以它为「窄」的下限。
    const NARROW_USABLE_WIDTH: f32 = 620.0;

    struct AgentInputLayoutRoot {
        input: Entity<AgentInput>,
        width: Pixels,
        height: Pixels,
    }

    impl AgentInputLayoutRoot {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self::with_width(px(NARROW_USABLE_WIDTH), window, cx)
        }

        fn wide(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self::with_width(px(900.0), window, cx)
        }

        fn running_with_queue(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let mut root = Self::with_width(px(NARROW_USABLE_WIDTH), window, cx);
            root.height = px(320.0);
            root.input.update(cx, |input, cx| {
                input.set_running(true, cx);
                input.set_queued_submissions(
                    vec![
                        QueuedPromptPreview::new("检查数据库连接状态", 0),
                        QueuedPromptPreview::new("", 2),
                    ],
                    cx,
                );
            });
            root
        }

        fn blocked_with_queue(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let mut root = Self::with_width(px(NARROW_USABLE_WIDTH), window, cx);
            root.height = px(320.0);
            root.input.update(cx, |input, cx| {
                input.set_pending_queue_blocked(true, cx);
                input.set_queued_submissions(
                    vec![QueuedPromptPreview::new("等待 ACP 恢复后执行", 0)],
                    cx,
                );
            });
            root
        }

        /// 注入一个特别长的权限文案，用于底栏截断测试。
        fn with_composer_context(
            width: Pixels,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            let root = Self::with_width(width, window, cx);
            root.input.update(cx, |input, cx| {
                let mut context = input.context.clone();
                context.execution_mode_label =
                    SharedString::from("允许完全访问并自动应用所有工具调用");
                input.set_context(context, cx);
            });
            root
        }

        fn with_width(width: Pixels, window: &mut Window, cx: &mut Context<Self>) -> Self {
            let input = cx.new(|cx| {
                AgentInput::with_mentions(Vec::new(), "描述目标，输入 @ 引用资源…", window, cx)
            });
            input.update(cx, |input, cx| {
                input.set_context(
                    AgentComposerContext {
                        model: Some(ComposerModel::new(
                            "Very Long Provider Name",
                            "extremely-long-model-name-with-large-context",
                        )),
                        execution_mode_label: SharedString::from("自动"),
                        ..AgentComposerContext::default()
                    },
                    cx,
                );
                input.set_menu_options(
                    vec![ComposerModelOption::new(
                        "long-model",
                        "provider",
                        "Very Long Provider Name",
                        "extremely-long-model-name-with-large-context",
                    )],
                    vec![
                        ComposerMenuOption::new("auto", "自动"),
                        ComposerMenuOption::new("readonly", "只读"),
                        ComposerMenuOption::new("manual", "手动确认"),
                    ],
                    cx,
                );
            });
            Self {
                input,
                width,
                height: px(220.0),
            }
        }
    }

    impl Render for AgentInputLayoutRoot {
        fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            // 测试窗口默认 rem = 16px；navop 生产环境的 rem 基准是主题字号
            // （默认 14px）。对齐到 14px，`sp()` 才会解析成书写时的像素值。
            window.set_rem_size(px(14.0));
            div().w(self.width).h(self.height).child(self.input.clone())
        }
    }

    /// 附件、模型、权限、发送都在输入卡片底部的**同一行**。
    /// 侧栏被拖窄时它们仍留在同一行：靠收缩 + 截断消化，绝不把发送按钮挤出卡片。
    #[gpui::test]
    fn composer_row_truncates_instead_of_pushing_the_send_button_out(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        const NARROW: f32 = 300.0;
        let (_, cx) = cx.add_window_view(|window, cx| {
            AgentInputLayoutRoot::with_composer_context(px(NARROW), window, cx)
        });
        let cx: &mut VisualTestContext = cx;

        let card = cx
            .debug_bounds("agent-input-card")
            .expect("input card should render");
        let model = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should render");
        let permission = cx
            .debug_bounds("agent-input-permission")
            .expect("permission chip should render");
        let send = cx
            .debug_bounds("agent-input-send-control")
            .expect("send control should render");

        // 发送按钮是这一行的锚点：任何收缩都不能把它推出卡片。
        assert!(
            send.origin.x + send.size.width <= card.origin.x + card.size.width,
            "send control must stay inside the card: send={send:?}, card={card:?}"
        );
        // 权限档位是内容宽控件，窄宽度下不能被压到读不出字。
        assert!(
            permission.size.width >= px(COMPOSER_CHIP_MIN_WIDTH),
            "permission chip must not shrink below the readable floor: {permission:?}"
        );
        // 收缩之后仍然是同一行。
        let row_y = model.origin.y;
        for bounds in [permission, send] {
            assert!(
                (bounds.origin.y - row_y).abs() <= px(2.0),
                "controls must stay on one row at narrow width: {bounds:?} (row_y={row_y:?})"
            );
        }
    }

    #[gpui::test]
    fn narrow_layout_keeps_model_and_send_controls_visible(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        let model = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should be rendered");
        let send = cx
            .debug_bounds("agent-input-send-control")
            .expect("send control should be rendered");

        assert!(model.size.width >= px(150.0));
        assert!(send.size.width >= px(28.0));
        assert!(
            model.origin.x + model.size.width <= send.origin.x,
            "model and send controls must not overlap: model={model:?}, send={send:?}"
        );
    }

    #[gpui::test]
    fn wide_layout_expands_model_control(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::wide);
        let cx: &mut VisualTestContext = cx;

        let model = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should be rendered");
        let send = cx
            .debug_bounds("agent-input-send-control")
            .expect("send control should be rendered");

        assert!(
            model.size.width >= px(280.0),
            "model control should use available toolbar width: model={model:?}"
        );
        assert_eq!(
            model.size.height, send.size.height,
            "model and send controls should align vertically: model={model:?}, send={send:?}"
        );
    }

    #[gpui::test]
    fn running_toolbar_keeps_queue_stop_and_model_controls_visible(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::running_with_queue);
        let cx: &mut VisualTestContext = cx;

        let model = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should be rendered");
        let queue = cx
            .debug_bounds("agent-input-queue-send")
            .expect("queue send control should be rendered");
        let stop = cx
            .debug_bounds("agent-input-stop")
            .expect("stop control should be rendered");

        assert!(model.size.width >= px(96.0));
        assert!(
            model.origin.x + model.size.width <= queue.origin.x,
            "model and queue controls must not overlap: model={model:?}, queue={queue:?}"
        );
        assert!(
            queue.origin.x + queue.size.width <= stop.origin.x,
            "queue and stop controls must not overlap: queue={queue:?}, stop={stop:?}"
        );
    }

    #[test]
    fn execution_trigger_width_tracks_label_length() {
        // 中文短标签收窄到下限,给模型选择与操作按钮让出空间。
        assert_eq!(execution_trigger_width("自动", false), sp(80.0));
        assert_eq!(execution_trigger_width("只读", false), sp(80.0));
        // 四字标签按内容展开,仍明显窄于旧的固定 124px。
        assert_eq!(execution_trigger_width("手动确认", false), sp(100.0));
        // 英文长标签不突破上限,保留原有的可用宽度。
        assert_eq!(
            execution_trigger_width("Manual Confirmation", false),
            sp(124.0)
        );
        // 运行中还要容纳排队与停止两个按钮,上限更低。
        let running = execution_trigger_width("自动", true);
        assert!(
            running.to_pixels(px(14.0)) < px(80.0),
            "running trigger should stay narrower: {running:?}"
        );
    }

    /// 「挤不下的项点开看全文」这条兜底,对模型下拉必须**真的**成立。
    ///
    /// 弹层宽度在 navop 侧没有直接观测点:组件库 `SelectOptions` 是私有的,弹层表面
    /// 没有调试选择器,选项行又按内容定宽(量行宽量到的是标签,不是弹层)。唯一可用
    /// 的位置效应是:`Positioner` 会把**放不进视口**的弹层从右缘往回压。所以先把窗口
    /// 收窄到与输入卡同宽,再点开弹层,断言选项行退到了触发器左侧 —— 只有弹层确实
    /// 比触发器宽(显式传的 320px,而非默认的「触发器多宽弹层多宽」)才会发生;
    /// 退回 `Length::Auto` 时弹层与触发器同宽,纹丝不动。
    #[gpui::test]
    fn model_menu_popup_is_wider_than_the_squeezed_trigger(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        // 底栏只剩模型与权限两项，620px 的窗口里模型根本不会被挤；要触发下限
        // 必须把卡片收到更窄——这里直接给到只剩两项也放不下的宽度。
        const SQUEEZED: f32 = 300.0;
        let (_, cx) = cx.add_window_view(|window, cx| {
            AgentInputLayoutRoot::with_composer_context(px(SQUEEZED), window, cx)
        });
        let cx: &mut VisualTestContext = cx;
        // 默认测试窗口比输入卡宽,320px 的弹层随便放、不会触发右缘收敛;
        // 收窄到同宽,「放不下」才真的发生。
        cx.simulate_resize(size(px(SQUEEZED), px(400.0)));

        let trigger = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should render");
        // 前置条件:这一行必须真的把触发器挤到下限,否则下面的断言证明不了兜底。
        assert!(
            trigger.size.width <= px(MODEL_TRIGGER_MIN_WIDTH + 1.0),
            "模型触发器应被挤到下限: trigger={trigger:?}, min={MODEL_TRIGGER_MIN_WIDTH}"
        );
        assert!(
            cx.debug_bounds("agent-model-option").is_none(),
            "弹层没打开时不该有选项行"
        );

        cx.simulate_click(trigger.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let option = cx.debug_bounds("agent-model-option").expect(
            "点击模型控件后弹层应打开并渲染出选项行(选择器在 model_picker::ModelChoice::render)",
        );
        assert!(
            option.origin.x < trigger.origin.x,
            "弹层没有比触发器宽 —— 显式 `menu_width` 丢了(退回 `Length::Auto` 时弹层与触发器 \
             同宽,不会被视口右缘往回压)。option={option:?}, trigger={trigger:?}"
        );
    }

    /// 常量层面的守卫:弹层下限必须高于触发器下限,且放得下长标签。
    ///
    /// 它只守「数值没被改坏」,守不住「`.menu_width(...)` 还在不在」——
    /// 那由 `model_menu_popup_is_wider_than_the_squeezed_trigger` 量真弹层来守。
    #[test]
    fn model_menu_stays_wider_than_a_squeezed_trigger() {
        assert!(
            MODEL_MENU_MIN_WIDTH > MODEL_TRIGGER_MIN_WIDTH,
            "弹层下限必须大于触发器下限: menu={MODEL_MENU_MIN_WIDTH}, trigger={MODEL_TRIGGER_MIN_WIDTH}"
        );
        // ACP agent 报上来的模型名常见就有这么长,弹层下限要能放下整个标签。
        let long = "9Router / deepseek-v3.1-terminus";
        assert!(
            MODEL_MENU_MIN_WIDTH >= estimated_label_width(long) + MODEL_MENU_CHROME_WIDTH,
            "弹层下限放不下长标签: {long}"
        );
    }

    /// 弹层宽度按最长选项估:短列表用下限(不塌成一条),超长收在上限内。
    #[test]
    fn model_menu_width_tracks_the_longest_option() {
        let short = ComposerModelOption::new("m1", "p1", "OpenAI", "gpt-4o-mini");
        assert_eq!(
            model_menu_width(std::slice::from_ref(&short)),
            sp(MODEL_MENU_MIN_WIDTH),
            "短标签用下限即可,不该比下限还窄"
        );

        let long = ComposerModelOption::new("m2", "p2", "p".repeat(40), "m".repeat(60));
        assert_eq!(
            model_menu_width(&[short, long]),
            sp(MODEL_MENU_MAX_WIDTH),
            "超长标签要收在上限内,否则浮层会被窗口边缘收敛"
        );
    }

    /// 有上下文用量读数时,工具栏右侧多一个圆环,且排在发送按钮左边。
    /// 没有读数时不渲染圆环——不摆一个空圈占位。
    /// 窗口已知时详情面板给四行：已用 / 上限 / 剩余 / 费用。
    ///
    /// 标签用同一批 `t!` 钥拼出来：测试环境不确定当前 locale，
    /// 这里真正要卡的是「哪几行、什么顺序、数值怎么格式化」。
    /// 窗口未知时不编造上限与剩余：只报已用，并明说上限未知。
    /// 点圆环展开详情面板；再点一次收起。
    /// 工具栏一行内同时有「附件 + 上下文档位 + 发送」；发送按钮仍是 32x32。
    #[gpui::test]
    fn toolbar_keeps_action_button_square_and_permission_in_the_same_row(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        let send = cx
            .debug_bounds("agent-send-button")
            .expect("send button should be rendered");
        assert_eq!(
            (send.size.width, send.size.height),
            (
                px(TOOLBAR_ACTION_BUTTON_SIZE),
                px(TOOLBAR_ACTION_BUTTON_SIZE)
            ),
            "send button must stay a 32x32 tap target: send={send:?}"
        );

        let toolbar = cx
            .debug_bounds("agent-input-toolbar")
            .expect("toolbar should render");
        let permission = cx
            .debug_bounds("agent-input-permission")
            .expect("permission chip should render in the toolbar");
        assert!(
            permission.origin.y >= toolbar.origin.y
                && permission.origin.y + permission.size.height
                    <= toolbar.origin.y + toolbar.size.height,
            "permission chip must share the toolbar row, not sit in a second row: \
             permission={permission:?}, toolbar={toolbar:?}"
        );
        assert!(
            permission.size.width <= px(124.0),
            "permission chip should hug its short label: permission={permission:?}"
        );
        assert!(
            permission.size.height <= px(TOOLBAR_ACTION_BUTTON_SIZE),
            "permission chip must not outgrow the toolbar buttons: permission={permission:?}"
        );
    }

    #[gpui::test]
    fn running_toolbar_keeps_action_buttons_square(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::running_with_queue);
        let cx: &mut VisualTestContext = cx;

        for id in ["agent-queue-send-button", "agent-stop-button"] {
            let bounds = cx
                .debug_bounds(id)
                .unwrap_or_else(|| panic!("{id} should be rendered"));
            assert_eq!(
                (bounds.size.width, bounds.size.height),
                (
                    px(TOOLBAR_ACTION_BUTTON_SIZE),
                    px(TOOLBAR_ACTION_BUTTON_SIZE)
                ),
                "{id} must stay a 32x32 tap target: {bounds:?}"
            );
        }
    }

    #[gpui::test]
    fn blocked_queue_keeps_queue_and_stop_controls_without_marking_turn_running(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (root, cx) = cx.add_window_view(AgentInputLayoutRoot::blocked_with_queue);
        let cx: &mut VisualTestContext = cx;

        assert!(cx.debug_bounds("agent-input-queue-send").is_some());
        assert!(cx.debug_bounds("agent-input-stop").is_some());
        assert!(cx.debug_bounds("agent-input-send-control").is_none());
        root.read_with(cx, |root, cx| {
            assert!(!root.input.read(cx).is_running());
        });
    }

    #[gpui::test]
    fn queued_submission_preview_renders_above_editor(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::running_with_queue);
        let cx: &mut VisualTestContext = cx;

        let queue = cx
            .debug_bounds("agent-input-queued")
            .expect("queued preview should be rendered");
        let item = cx
            .debug_bounds("agent-input-queued-item")
            .expect("queued preview item should be rendered");
        let editor = cx
            .debug_bounds("agent-input-editor")
            .expect("editor should be rendered");

        assert!(item.size.height > px(0.0));
        assert!(
            queue.origin.y + queue.size.height <= editor.origin.y,
            "queued preview must stay above the editor: queue={queue:?}, editor={editor:?}"
        );
    }

    #[gpui::test]
    fn mention_completion_popup_stays_above_bottom_toolbar(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        let input = cx
            .debug_bounds("agent-input-root")
            .expect("input root should render");
        let toolbar = cx
            .debug_bounds("agent-input-toolbar")
            .expect("toolbar should render");

        assert!(
            input.origin.y < toolbar.origin.y,
            "input editor should have vertical room above toolbar for completion popup"
        );
    }

    #[test]
    fn merged_mode_menu_keeps_all_tool_confirmation_options() {
        let options = vec![
            ComposerMenuOption::new("auto", "自动"),
            ComposerMenuOption::new("readonly", "只读"),
            ComposerMenuOption::new("manual", "手动确认"),
        ];

        assert_eq!(
            vec!["自动", "只读", "手动确认"],
            options
                .iter()
                .map(|option| option.label.as_ref())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_execution_mode_label_defaults_to_auto() {
        assert_eq!(
            t!("AgentUi.auto").as_ref(),
            current_execution_mode_label(&SharedString::from("")).as_ref()
        );
    }

    #[test]
    fn execution_mode_trigger_uses_single_label() {
        assert_eq!(
            "只读",
            current_execution_mode_label(&SharedString::from("只读")).as_ref()
        );
    }

    #[test]
    fn history_navigation_only_uses_unmodified_vertical_keys() {
        assert_eq!(
            Some(HistoryDirection::Previous),
            history_direction("up", &Modifiers::default())
        );
        assert_eq!(
            Some(HistoryDirection::Next),
            history_direction("down", &Modifiers::default())
        );
        assert_eq!(None, history_direction("left", &Modifiers::default()));
        assert_eq!(
            None,
            history_direction(
                "up",
                &Modifiers {
                    shift: true,
                    ..Modifiers::default()
                }
            )
        );
    }

    #[test]
    fn history_navigation_only_uses_document_boundaries() {
        let text = "first\nsecond\nthird";

        assert!(cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            0
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            "fir".len()
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            "first\nsec".len()
        ));
        assert!(cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            text.len()
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            "first\nsecond\nth".len()
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            "first\nsec".len()
        ));
    }

    #[test]
    fn history_navigation_uses_utf8_byte_offsets_at_document_boundaries() {
        let text = "第一行\nsecond\n第三行";

        assert!(cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            0
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            "第".len()
        ));
        assert!(cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            text.len()
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            text.len() - "行".len()
        ));
    }

    #[test]
    fn referenced_mentions_do_not_match_label_prefixes() {
        let mentions = vec![
            MentionItem::new("short", "prod", "mysql", "mysql"),
            MentionItem::new("long", "prod-db", "mysql", "mysql"),
        ];

        let got = referenced_mentions_in_text("分析 @`prod-db` 慢查询", &mentions);

        assert_eq!(
            vec!["long"],
            got.iter().map(|item| item.id.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn referenced_mentions_match_quoted_names_with_spaces() {
        let mentions = vec![MentionItem::new("c1", "prod db", "postgres", "postgres")];

        let got = referenced_mentions_in_text("请检查 @`prod db` 的连接数", &mentions);

        assert_eq!(
            vec!["c1"],
            got.iter().map(|item| item.id.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn referenced_mentions_respect_simple_name_boundaries() {
        let mentions = vec![MentionItem::new("c1", "prod", "mysql", "mysql")];

        let got = referenced_mentions_in_text("请检查 @production", &mentions);

        assert!(got.is_empty());
    }

    /// 点缩略图开预览、再点一下关掉；关掉后 `previewed_attachment` 必须回到
    /// `None`，否则覆盖层会带进下一个会话。
    #[gpui::test]
    fn attachment_thumbnail_click_toggles_preview(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (root, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        let attachment = ImageAttachment::for_test("shot.png");
        let attachment_id = attachment.id.clone();
        let input = root.read_with(cx, |root, _| root.input.clone());
        input.update(cx, |input, cx| {
            input.set_composer_attachments(vec![attachment], cx);
        });
        cx.run_until_parked();

        // `debug_bounds` 要 `&'static str`；测试里泄漏一个选择器字符串最省事。
        let selector: &'static str =
            Box::leak(format!("agent-attachment-{attachment_id}").into_boxed_str());
        let thumb = cx.debug_bounds(selector).expect("缩略图应该渲染出来");
        cx.simulate_click(thumb.center(), Modifiers::default());
        assert!(
            input.read_with(cx, |input, _| input.previewed_attachment().is_some()),
            "点缩略图应该打开预览"
        );

        cx.run_until_parked();
        let thumb = cx.debug_bounds(selector).expect("缩略图仍然在");
        cx.simulate_click(thumb.center(), Modifiers::default());
        assert!(
            input.read_with(cx, |input, _| input.previewed_attachment().is_none()),
            "再点一下应该关掉预览"
        );
    }

    /// 删掉附件时预览必须一起关：否则覆盖层会留着一张已经不在列表里的图。
    #[gpui::test]
    fn removing_the_previewed_attachment_closes_the_preview(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (root, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        let attachment = ImageAttachment::for_test("shot.png");
        let attachment_id = attachment.id.clone();
        let input = root.read_with(cx, |root, _| root.input.clone());
        input.update(cx, |input, cx| {
            input.set_composer_attachments(vec![attachment], cx);
            input.toggle_attachment_preview(&attachment_id, cx);
        });
        assert!(input.read_with(cx, |input, _| input.previewed_attachment().is_some()));

        input.update(cx, |input, cx| {
            input.remove_attachment(&attachment_id, cx);
        });
        assert!(input.read_with(cx, |input, _| input.previewed_attachment().is_none()));
    }

    /// 从 Finder 拖进来的文件分两路：图片成附件，其余把路径插到正文里。
    ///
    /// 非图片不做附件——附件最终是作为多模态图片发给模型的，塞一个 PDF 进去
    /// 只会在提交时报错；路径写进正文，agent 自己去读更靠谱。
    #[gpui::test]
    fn dropped_files_split_into_attachments_and_paths(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (root, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        // 真文件：拖放路径来自操作系统，构造出来的假路径读不出字节。
        let dir = std::env::temp_dir().join(format!("navop-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let image_path = dir.join("shot.png");
        std::fs::write(&image_path, [137u8, 80, 78, 71]).expect("write png");
        let text_path = dir.join("notes.txt");
        std::fs::write(&text_path, b"hello").expect("write txt");

        root.update_in(cx, |root, window, cx| {
            root.input.update(cx, |input, cx| {
                input.add_dropped_paths(&[image_path.clone(), text_path.clone()], window, cx);
            });
        });

        root.read_with(cx, |root, cx| {
            let input = root.input.read(cx);
            let attachments = input.composer_attachments();
            assert_eq!(1, attachments.len(), "图片落成一张附件");
            assert_eq!("shot.png", attachments[0].name);

            let text = input.composer_text(cx);
            assert!(
                text.contains("notes.txt"),
                "非图片该把路径写进正文，实际是 {text:?}"
            );
            assert!(
                !text.contains("shot.png"),
                "图片走了附件，不该再重复写一份路径：{text:?}"
            );
        });

        let _ = std::fs::remove_dir_all(&dir);
    }
}
