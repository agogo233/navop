use std::{rc::Rc, sync::Arc, sync::atomic::AtomicBool, sync::atomic::Ordering};

use gpui::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, WindowExt as _,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    dialog::{Dialog, DialogFooter},
    h_flex, v_flex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileConflictChoice {
    Skip,
    KeepBoth,
    Merge,
    Overwrite,
}

/// 文件夹冲突里的「覆盖」是整目录替换，目标目录中独有的文件会一起消失；
/// 它和「合并」的区别最容易被误选，因此执行前必须让用户再确认一次。
pub fn requires_overwrite_confirmation(choice: FileConflictChoice, is_directory: bool) -> bool {
    is_directory && choice == FileConflictChoice::Overwrite
}

#[derive(Clone)]
pub struct FileConflictPromptLabels {
    pub exists: SharedString,
    pub progress: SharedString,
    pub choose_action: SharedString,
    pub apply_all: SharedString,
    pub skip: SharedString,
    pub keep_both: SharedString,
    pub merge: SharedString,
    pub overwrite: SharedString,
    /// 「覆盖」风险确认弹窗的标题。
    pub overwrite_warning_title: SharedString,
    /// 「覆盖」风险确认弹窗的正文。
    pub overwrite_warning: SharedString,
    /// 已勾选「应用到所有同类冲突」时的追加提醒。
    pub overwrite_warning_apply_all: SharedString,
    /// 「仍然覆盖」确认按钮。
    pub overwrite_confirm: SharedString,
    /// 取消确认按钮。
    pub cancel: SharedString,
}

type ChoiceHandler = Rc<dyn Fn(FileConflictChoice, bool, &mut Window, &mut App)>;

pub struct FileConflictPromptSpec {
    pub name: SharedString,
    pub is_directory: bool,
    pub apply_all: Arc<AtomicBool>,
    pub labels: FileConflictPromptLabels,
}

#[derive(IntoElement)]
pub struct FileConflictPrompt {
    name: SharedString,
    is_directory: bool,
    apply_all: Arc<AtomicBool>,
    labels: FileConflictPromptLabels,
    on_choice: ChoiceHandler,
}

impl FileConflictPrompt {
    pub fn new(
        spec: FileConflictPromptSpec,
        on_choice: impl Fn(FileConflictChoice, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            name: spec.name,
            is_directory: spec.is_directory,
            apply_all: spec.apply_all,
            labels: spec.labels,
            on_choice: Rc::new(on_choice),
        }
    }

    fn action_button(&self, choice: FileConflictChoice, label: SharedString) -> Button {
        let apply_all = self.apply_all.clone();
        let on_choice = self.on_choice.clone();
        let button = Button::new(format!("file-conflict-{choice:?}")).label(label);
        if requires_overwrite_confirmation(choice, self.is_directory) {
            let warning = OverwriteWarning {
                name: self.name.clone(),
                labels: self.labels.clone(),
                apply_all,
                on_choice,
            };
            return button.on_click(move |_, window, cx| warning.open(window, cx));
        }
        button.on_click(move |_, window, cx| {
            on_choice(choice, apply_all.load(Ordering::Relaxed), window, cx);
        })
    }
}

impl RenderOnce for FileConflictPrompt {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(self.labels.exists.clone())
            .child(self.info_card(cx))
            .child(self.labels.choose_action.clone())
            .child(self.apply_all_checkbox())
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .children(self.action_buttons()),
            )
    }
}

impl FileConflictPrompt {
    /// 冲突项的名字与「第几个 / 共几个」进度。
    fn info_card(&self, cx: &App) -> impl IntoElement {
        div()
            .p_2()
            .rounded_md()
            .bg(cx.theme().secondary)
            .overflow_hidden()
            .child(
                div()
                    .text_sm()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.name.clone()),
            )
            .child(
                div()
                    .mt_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.labels.progress.clone()),
            )
    }

    fn apply_all_checkbox(&self) -> impl IntoElement {
        let checked = self.apply_all.load(Ordering::Relaxed);
        let apply_all = self.apply_all.clone();
        Checkbox::new("file-conflict-apply-all")
            .checked(checked)
            .label(self.labels.apply_all.clone())
            .on_click(move |checked, window, _cx| {
                apply_all.store(*checked, Ordering::Relaxed);
                window.refresh();
            })
    }

    fn action_buttons(&self) -> Vec<Button> {
        let mut actions = vec![
            self.action_button(FileConflictChoice::Skip, self.labels.skip.clone())
                .ghost(),
            self.action_button(FileConflictChoice::KeepBoth, self.labels.keep_both.clone())
                .ghost(),
        ];
        if self.is_directory {
            actions.push(
                self.action_button(FileConflictChoice::Merge, self.labels.merge.clone())
                    .ghost(),
            );
        }
        actions.push(
            self.action_button(FileConflictChoice::Overwrite, self.labels.overwrite.clone())
                .primary(),
        );
        actions
    }
}

/// 目录冲突选择「覆盖」后叠加的二次确认框，说清楚整目录替换会丢掉什么。
struct OverwriteWarning {
    name: SharedString,
    labels: FileConflictPromptLabels,
    apply_all: Arc<AtomicBool>,
    on_choice: ChoiceHandler,
}

impl OverwriteWarning {
    fn open(&self, window: &mut Window, cx: &mut App) {
        let warning = Self {
            name: self.name.clone(),
            labels: self.labels.clone(),
            apply_all: self.apply_all.clone(),
            on_choice: self.on_choice.clone(),
        };
        window.open_dialog(cx, move |dialog, _window, cx| warning.render(dialog, cx));
    }

    fn render(&self, dialog: Dialog, cx: &App) -> Dialog {
        dialog
            .title(self.labels.overwrite_warning_title.clone())
            .w(px(460.))
            .child(self.details(cx))
            .footer(self.footer())
            .overlay_closable(false)
            .close_button(true)
    }

    /// 正文：覆盖的风险说明、当前文件夹名，以及勾选「应用全部」时的追加提醒。
    fn details(&self, cx: &App) -> impl IntoElement {
        let labels = &self.labels;
        let mut content = v_flex()
            .gap_3()
            .child(div().text_sm().child(labels.overwrite_warning.clone()));
        content = content.child(
            div()
                .p_2()
                .rounded_md()
                .bg(cx.theme().secondary)
                .overflow_hidden()
                .child(
                    div()
                        .text_sm()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(self.name.clone()),
                ),
        );
        if self.apply_all.load(Ordering::Relaxed) {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(labels.overwrite_warning_apply_all.clone()),
            );
        }
        content
    }

    fn footer(&self) -> impl IntoElement {
        let labels = &self.labels;
        let apply_all = self.apply_all.clone();
        let on_choice = self.on_choice.clone();
        DialogFooter::new().children(vec![
            Button::new("file-conflict-overwrite-cancel")
                .label(labels.cancel.clone())
                .ghost()
                .on_click(|_, window, cx| window.close_dialog(cx))
                .into_any_element(),
            Button::new("file-conflict-overwrite-confirm")
                .label(labels.overwrite_confirm.clone())
                .danger()
                .on_click(move |_, window, cx| {
                    let apply_all = apply_all.load(Ordering::Relaxed);
                    window.close_dialog(cx);
                    on_choice(FileConflictChoice::Overwrite, apply_all, window, cx);
                })
                .into_any_element(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_overwrite_requires_confirmation() {
        assert!(requires_overwrite_confirmation(
            FileConflictChoice::Overwrite,
            true
        ));
    }

    #[test]
    fn file_overwrite_skips_confirmation() {
        assert!(!requires_overwrite_confirmation(
            FileConflictChoice::Overwrite,
            false
        ));
    }

    #[test]
    fn other_choices_skip_confirmation() {
        for choice in [
            FileConflictChoice::Skip,
            FileConflictChoice::KeepBoth,
            FileConflictChoice::Merge,
        ] {
            assert!(!requires_overwrite_confirmation(choice, true));
        }
    }
}
