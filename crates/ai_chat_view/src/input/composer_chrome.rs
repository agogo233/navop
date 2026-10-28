//! 两份输入框（One agent 与 ACP）共用的**版式骨架**。
//!
//! 这里只放「一块 composer 长什么样」：卡片的上下内边距与圆角、编辑区内框的底色
//! 与描边、参数行 / 上下文行的行高与内边距、chip 的尺寸与字号、发送键的边长。
//! 两份组件各自的**状态、入口与子面板**仍留在自己文件里 —— 共用的是尺子，不是组件。
//!
//! 为什么单独拆一个模块：这两份输入框上一次「看起来不一样」，不是因为谁更好看，
//! 而是因为同一组数值各写了一份，改了一边忘了另一边（26 高 vs 32 高、卡片 py=8 vs
//! py=0、编辑区内框 `theme.inset` vs 全局 `input_background()`）。数值从此只在这里
//! 定义一次，两份组件都从这里取。
//!
//! 两份组件仍然保持独立渲染树与独立状态：One agent 是「驾驶舱」（能力栏 + 资源池），
//! ACP 是「对讲机」（只用 agent 自己声明的东西）；共享的只有尺寸与配色语义。

use gpui::{
    App, Div, Entity, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, Rems,
    SharedString, Stateful, StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_component::searchable_list::SearchableVec;
use gpui_component::select::{Select, SelectGroup, SelectState};
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;

use crate::input::context::ComposerModelOption;
use crate::input::model_picker::ModelChoice;
use crate::theme::{AgentChatTheme, sp};

/// 卡片上下内边距（waku composer 的实测值）。
pub(crate) const CARD_PADDING_VERTICAL: f32 = 8.0;
/// 卡片圆角相对主题半径的倍数。
pub(crate) const CARD_RADIUS_FACTOR: f32 = 2.0;
/// 参数行与上下文行的水平内边距。
pub(crate) const ROW_PADDING_X: f32 = 10.0;
/// 参数行与编辑区之间的间距。
pub(crate) const TOOLBAR_MARGIN_TOP: f32 = 8.0;
/// 参数行里控件之间的间距。
pub(crate) const TOOLBAR_GAP: f32 = 4.0;
/// 编辑区上内边距。
pub(crate) const EDITOR_PADDING_TOP: f32 = 2.0;
/// 编辑区的最大高度（约十行）。
pub(crate) const EDITOR_MAX_HEIGHT: f32 = 220.0;
/// 所有 chip（含发送键）的高度；waku composer 的实测值。
///
/// 整块卡片里没有第二种高度：参数行的控件、上下文行的控件、发送键都对齐这一条。
pub(crate) const CHIP_HEIGHT: f32 = 26.0;
/// chip 的圆角：比卡片小一档（卡片 `radius * 2`），与组件库控件圆角同值。
pub(crate) const CHIP_RADIUS: f32 = 6.0;
/// 发送 / 排队 / 停止键的边长，与 chip 同高。
pub(crate) const ACTION_BUTTON_SIZE: f32 = CHIP_HEIGHT;
/// chip 里标签与图标共用的字号基准：同一行里不该有第二种字号。
pub(crate) const CHIP_FONT_SIZE: f32 = 12.5;
/// 可收缩 chip 的宽度下限（标签走省略号）。
pub(crate) const CHIP_MIN_WIDTH: f32 = 44.0;
/// 可收缩 chip 的宽度上限。
pub(crate) const CHIP_MAX_WIDTH: f32 = 220.0;
/// 模型 chip 的宽度上限：模型名长起来没边，超了就省略（全名进 tooltip）。
pub(crate) const MODEL_TRIGGER_MAX_WIDTH: f32 = 200.0;
/// 能力 chip（计划 / 子代理 / 上下文 / Skill）的宽度上限。
pub(crate) const CAPABILITY_CHIP_MAX_WIDTH: f32 = 180.0;
/// 模型下拉弹层的宽度区间（弹层不跟着被挤扁的触发器缩）。
pub(crate) const MODEL_MENU_MIN_WIDTH: f32 = 320.0;
pub(crate) const MODEL_MENU_MAX_WIDTH: f32 = 400.0;
pub(crate) const MODEL_MENU_CHROME_WIDTH: f32 = 56.0;
/// 上下文行的高度。
pub(crate) const CONTEXT_ROW_HEIGHT: f32 = 28.0;
/// 上下文行的上下内边距。
pub(crate) const CONTEXT_ROW_PADDING_VERTICAL: f32 = 1.0;
/// 上下文行里 chip 之间的间距（比参数行更紧，读起来是「贴着卡片的一行注释」）。
pub(crate) const CONTEXT_ROW_GAP: f32 = 2.0;
/// composer 与面板底边之间的留白（sidebar 模式）。
///
/// 侧边栏里的 composer 不是浮动卡片，它贴着面板底边：没有这段留白时，最后一行
/// 会紧贴窗口下沿，看起来像被裁掉半行。
pub(crate) const CONTAINER_PADDING_BOTTOM: f32 = 8.0;

/// composer 根容器：卡片 +（ACP 才有的）上下文行都挂在它下面。
///
/// 两份组件各传各的 `debug_selector`，其余写法保持一致：`flex_shrink_0` 让它在
/// 纵向布局里保持自身高度，`min_h_0` 允许被压缩时内部自己滚动（sidebar 模式）。
pub(crate) fn composer_root(
    theme: &AgentChatTheme,
    edge_to_edge: bool,
    debug_selector: &'static str,
) -> Stateful<Div> {
    v_flex()
        .id("agent-input-root")
        .debug_selector(move || debug_selector.to_string())
        .w_full()
        .min_w_0()
        .when(edge_to_edge, |this| this.min_h_0().flex_shrink_0())
        .when(!edge_to_edge, |this| this.flex_shrink_0())
        .bg(theme.background)
        .text_color(theme.foreground)
}

/// 卡片：composer 的可见边界。编辑器与参数行都在里面。
///
/// `edge_to_edge` 时去掉外框（圆角 / 描边 / 阴影）—— 侧栏里的 composer 用分隔线
/// 与上方消息列表分开，不再套一层卡片框。
pub(crate) fn composer_card(theme: &AgentChatTheme, edge_to_edge: bool) -> Div {
    v_flex()
        .debug_selector(|| "agent-input-card".to_string())
        .w_full()
        .min_w_0()
        .py(sp(CARD_PADDING_VERTICAL))
        .bg(theme.background)
        .text_color(theme.foreground)
        .when(!edge_to_edge, |this| {
            this.rounded(theme.surface_radius * CARD_RADIUS_FACTOR)
                .border_1()
                .border_color(theme.border)
                .shadow_sm()
        })
        .when(edge_to_edge, |this| this.min_h_0())
}

/// 编辑区：内框 + 编辑器本体。
///
/// 底色随卡片的形态变，不是两种风格：
/// - 有卡片框时（应用内的工作台），编辑区就是卡片本身的一块空地，再铺一层底
///   会变成「白卡片里套灰卡片」，所以不填色，只留描边；
/// - `edge_to_edge` 时（侧栏里没有卡片框），内框是唯一的分界线，用 `theme.inset`
///   打底 —— 它是面板调色板里的「内嵌一层」，深色面板取深色、浅色面板取浅灰。
///
/// 两处都不用全局 `input_background()`：它在浅色应用主题下等于背景色（白），
/// 落进深色侧栏就是一块白。
pub(crate) fn composer_editor_frame(
    theme: &AgentChatTheme,
    edge_to_edge: bool,
    focused: bool,
    editor: impl IntoElement,
) -> Div {
    div()
        .debug_selector(|| "agent-input-editor".to_string())
        .w_full()
        .min_w_0()
        .px_3()
        .pt(px(EDITOR_PADDING_TOP))
        .max_h(sp(EDITOR_MAX_HEIGHT))
        .child(
            div()
                .w_full()
                .min_w_0()
                .rounded(theme.surface_radius)
                .border_1()
                .border_color(if focused { theme.accent } else { theme.border })
                .bg(composer_editor_fill(theme, edge_to_edge))
                .child(editor),
        )
}

/// 编辑区内框的底色：卡片有框时不填色（跟卡片一个底色），`edge_to_edge` 时
/// 用面板的 `inset` 表面。见 [`composer_editor_frame`] 的说明。
pub(crate) fn composer_editor_fill(theme: &AgentChatTheme, edge_to_edge: bool) -> Hsla {
    if edge_to_edge {
        theme.inset
    } else {
        theme.background
    }
}

/// 能力行：卡片顶部一行 `[计划][子代理][上下文][Skill]`。
///
/// 与参数行分家的原因不是好看，是窄侧栏里挤不下：四颗 chip 和模型 / 权限 /
/// 附件 / 发送键同处一行时，两边一起退到 `CHIP_MIN_WIDTH`，标签只剩省略号
/// —— 那才是「显示不全」。分成两行后，能力行有整行宽度把标签写全，参数行
/// 专心放「这一轮怎么发出去」。
pub(crate) fn composer_capability_row(theme: &AgentChatTheme) -> Div {
    h_flex()
        .debug_selector(|| "agent-input-capability-row".to_string())
        .w_full()
        .min_w_0()
        .items_center()
        .text_color(theme.foreground)
        .gap(sp(TOOLBAR_GAP))
        .px(sp(ROW_PADDING_X))
        .pb(sp(2.0))
        .overflow_hidden()
        .flex_shrink_0()
}

/// 参数行：`[模型][模式][权限][附件][发送]`。
pub(crate) fn composer_toolbar(theme: &AgentChatTheme) -> Div {
    h_flex()
        .debug_selector(|| "agent-input-toolbar".to_string())
        .w_full()
        .min_w_0()
        .items_center()
        .text_color(theme.foreground)
        .gap(sp(TOOLBAR_GAP))
        .px(sp(ROW_PADDING_X))
        .mt(sp(TOOLBAR_MARGIN_TOP))
        // 极端窄宽度下宁可裁掉中段的尾巴，也不让内容画到输入卡片外面。
        .overflow_hidden()
        .flex_shrink_0()
}

/// 卡片外的上下文行：`[工作区][分支][Worktree] … [用量]`。
///
/// 这一行回答的是「这次跑在哪」，与卡片里「这次怎么跑」不是一件事；行高 28、
/// 内边距 10、组内间距 2 —— waku 那一行的实测值，比卡片内部松、比卡片外围紧。
pub(crate) fn composer_context_row(theme: &AgentChatTheme) -> Div {
    h_flex()
        .debug_selector(|| "agent-input-context-row".to_string())
        .w_full()
        .min_w_0()
        .min_h(sp(CONTEXT_ROW_HEIGHT))
        .items_center()
        .gap(sp(CONTEXT_ROW_GAP))
        .py(sp(CONTEXT_ROW_PADDING_VERTICAL))
        .px(sp(ROW_PADDING_X))
        .flex_shrink_0()
        .text_size(sp(CHIP_FONT_SIZE))
        .text_color(theme.muted_foreground)
}

/// 下拉菜单的表面：浮在卡片上的一块浮层，不是一块「隐形」的列表。
///
/// gpui 的 `Popover` 只负责定位，内容容器不画底色；调用方若只写文字与行 hover，
/// 菜单就会直接叠在面板上（深色侧栏里尤其像没画完）。表面色取调色板里的
/// `raised`，跟着面板配色走，而不是应用主题的 `popover`。
pub(crate) fn composer_popover_surface(theme: &AgentChatTheme) -> Div {
    v_flex()
        .debug_selector(|| "agent-input-popover-surface".to_string())
        .p_1()
        .gap(sp(2.0))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.surface_radius)
        .shadow_md()
        .text_color(theme.foreground)
}

/// chip 的内容：`[图标] [可截断标签]`。
///
/// 不带 `⌄`：整块卡片原来有六条小箭头，一起消失是减线里最见效的一步。
/// 「可点」改由 hover 底色与 tooltip 表达。
///
/// 字号与图标尺寸跟着**同一行**的基准走：标签 `sp(CHIP_FONT_SIZE)`、图标 `.small()`。
/// 早前标签写 `text_xs()`、图标写 `.xsmall()`，两样都比同一行里的身份行小一档多，
/// 参数行与上下文行因此读起来像「附注」，而模型那串反而成了最大的字。
pub(crate) fn composer_chip_label(
    icon: IconName,
    icon_color: Hsla,
    label: SharedString,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_1()
        .child(
            Icon::new(icon)
                .small()
                .flex_shrink_0()
                .text_color(icon_color),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(sp(CHIP_FONT_SIZE))
                .child(label),
        )
}

/// 参数行统一的 ghost chip：无描边、无底色，hover 才浮一层 `overlay`。
///
/// 组件库的 `Button::ghost()` 拿 accent 当 hover，铺在整行里太抢眼；这里用
/// `custom` 把底色钉成中性 overlay，与其余自绘 chip 同一种反馈（去掉的是线，
/// 不是反馈）。
pub(crate) fn ghost_chip(button: Button, theme: &AgentChatTheme, cx: &App) -> Button {
    ghost_chip_with(button, theme.foreground, theme, cx)
}

/// 上下文行的 ghost chip：整行是元数据，文字压到次要色 —— 它不该跟参数行的
/// 控件（模型 / 模式 / 发送）抢语气。
pub(crate) fn ghost_chip_muted(button: Button, theme: &AgentChatTheme, cx: &App) -> Button {
    ghost_chip_with(button, theme.muted_foreground, theme, cx)
}

pub(crate) fn ghost_chip_with(
    button: Button,
    foreground: Hsla,
    theme: &AgentChatTheme,
    cx: &App,
) -> Button {
    button
        .h(sp(CHIP_HEIGHT))
        .rounded(px(CHIP_RADIUS))
        // 组件库的按钮默认是箭头光标（桌面惯例），但参数行这些 chip 在鼠标悬停时
        // 什么反馈都给不出「能点」的暗示，手指光标是这里唯一的提示。
        .cursor_pointer()
        .custom(
            ButtonCustomVariant::new(cx)
                .foreground(foreground)
                .hover(theme.overlay)
                .active(theme.overlay),
        )
}

/// 发送 / 排队 / 停止键的边长（与 chip 同高）。
pub(crate) fn action_button_size() -> Rems {
    sp(ACTION_BUTTON_SIZE)
}

/// 发送 / 排队 / 停止键的圆角：正好半边长，画成圆。
pub(crate) fn action_button_radius() -> Pixels {
    px(ACTION_BUTTON_SIZE / 2.0)
}

/// 能力 chip（计划 / 子代理 / 上下文 / 技能）在参数行里的占位。
///
/// 参数行是一条不换行的横排：四颗能力 chip 先按内容取宽，挤不下时由外框统一收缩到
/// `CHIP_MIN_WIDTH`，标签自己走省略号 —— 发送键与模型 chip 的位置由外框收缩保住，
/// 不会像之前那样被 chip 顶出卡片。
pub(crate) fn composer_capability_slot() -> Div {
    div()
        .min_w(sp(CHIP_MIN_WIDTH))
        .max_w(sp(CAPABILITY_CHIP_MAX_WIDTH))
        .h(sp(CHIP_HEIGHT))
        .flex_shrink(1.0)
        .overflow_hidden()
}

/// 权限 chip 的占位：比能力 chip 宽一档（`CHIP_MAX_WIDTH`），同样可收缩。
pub(crate) fn composer_permission_slot() -> Div {
    div()
        .min_w(sp(CHIP_MIN_WIDTH))
        .max_w(sp(CHIP_MAX_WIDTH))
        .h(sp(CHIP_HEIGHT))
        .flex_shrink(1.0)
        .overflow_hidden()
}

/// 能力 chip（计划 / 子代理 / 上下文 / Skill）：26 高的 ghost chip，宽度随内容。
///
/// 两份输入框的能力 chip 用同一个实现：它们回答的是「这一轮能用到什么」，本来就
/// 该长一个样。标签截断时 tooltip 给出全文。
pub(crate) fn capability_chip(
    id: &'static str,
    label: impl Into<SharedString>,
    icon: IconName,
    foreground: Hsla,
    theme: &AgentChatTheme,
    cx: &App,
) -> Button {
    let label = label.into();
    let tooltip = label.clone();
    ghost_chip(
        Button::new(id)
            .debug_selector(move || id.to_string())
            .small()
            .min_w_0()
            .max_w(sp(CAPABILITY_CHIP_MAX_WIDTH))
            .flex_shrink(1.0)
            .tooltip(tooltip)
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .text_size(sp(CHIP_FONT_SIZE))
                    .text_color(foreground)
                    .child(Icon::new(icon).small().flex_shrink_0())
                    .child(div().truncate().child(label)),
            ),
        theme,
        cx,
    )
}

/// 模型 chip：组件库 `Select` 的透明形态 + 一层 hover 壳。
///
/// `Select` 在 `appearance(false)` 下不画 hover（连 focus ring 也只在 `appearance`
/// 为真时才画），所以外面套一层带 hover 的壳；尾标用模型图标而不是 `⌄`，与其余
/// chip 的「无箭头」保持一致。名字被省略时 tooltip 给全名。
#[allow(clippy::too_many_arguments)]
pub(crate) fn composer_model_chip(
    select: &Entity<SelectState<SearchableVec<SelectGroup<ModelChoice>>>>,
    container_id: &'static str,
    select_id: &'static str,
    debug_selector: &'static str,
    placeholder: SharedString,
    menu_width: Rems,
    disabled: bool,
    theme: &AgentChatTheme,
) -> Stateful<Div> {
    div()
        .id(container_id)
        .debug_selector(move || debug_selector.to_string())
        .flex_shrink(1.0)
        .max_w(sp(MODEL_TRIGGER_MAX_WIDTH))
        .h(sp(CHIP_HEIGHT))
        .rounded(px(CHIP_RADIUS))
        .overflow_hidden()
        .cursor_pointer()
        .hover(|this| this.bg(theme.overlay))
        .text_color(theme.foreground)
        .tooltip({
            let full = placeholder.clone();
            move |window, cx| Tooltip::new(full.clone()).build(window, cx)
        })
        .child(
            Select::new(select)
                .id(select_id)
                .w_full()
                .h(sp(CHIP_HEIGHT))
                .small()
                .icon(IconName::AILine)
                .appearance(false)
                .placeholder(placeholder)
                .search_placeholder(t!("AgentUi.search_model").to_string())
                .menu_width(menu_width)
                .menu_max_h(sp(320.0))
                .disabled(disabled),
        )
}

/// 模型下拉弹层的宽度：按最长的选项估，保证点开能读到完整的候选名。
pub(crate) fn model_menu_width(options: &[ComposerModelOption]) -> Rems {
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

/// 估算一段文案的渲染宽度：CJK 等全角字符按 13px，其余按 7.5px。
pub(crate) fn estimated_label_width(label: &str) -> f32 {
    label
        .chars()
        .map(|ch| if is_full_width_char(ch) { 13.0 } else { 7.5 })
        .sum()
}

/// 常见全角字符区间（中日韩文字、全角标点等）。
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    /// 编辑区底色只有两条规则：卡片有框时不填色，`edge_to_edge` 时用面板的
    /// `inset`。这张截图级反馈（「卡片里套了一块灰」）就靠这条卡住。
    #[gpui::test]
    fn editor_fill_follows_card_form(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            let theme = AgentChatTheme::from_app(cx);

            assert_eq!(theme.background, composer_editor_fill(&theme, false));
            assert_eq!(theme.inset, composer_editor_fill(&theme, true));
            assert_ne!(
                theme.background, theme.inset,
                "`inset` 与卡片底色必须能区分，否则侧栏里的内框就消失了"
            );
        });
    }
}
