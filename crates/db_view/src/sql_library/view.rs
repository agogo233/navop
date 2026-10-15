//! SQL 库面板的渲染：头部工具栏 + 展平后的树行。

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{ActiveTheme, Icon, Sizable as _, Size, h_flex, v_flex};
use one_assets::IconName;
use one_ui::{IconButton, IconSize as OneIconSize};
use rust_i18n::t;
use std::path::{Path, PathBuf};

use super::panel::SqlLibraryPanel;
use super::tree::SqlLibraryRow;

const ROW_HEIGHT: f32 = 24.;
const INDENT_STEP: f32 = 12.;
const ACTION_BUTTON_SIZE: f32 = 20.;

/// 头部：标题 + 新建文件 / 新建目录 / 挂载目录 / 刷新。
pub(super) fn render_header(
    target: Option<PathBuf>,
    cx: &mut Context<SqlLibraryPanel>,
) -> AnyElement {
    let target_hint = target
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();

    h_flex()
        .w_full()
        .h(px(32.))
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap_1()
        .px_2()
        .child(
            h_flex()
                .min_w_0()
                .items_center()
                .gap_1()
                .child(
                    Icon::new(IconName::SqlLibrary)
                        .with_size(px(14.))
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(t!("SqlLibrary.title").to_string()),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .child(header_button(
                    "sql-library-new-file",
                    IconName::Plus,
                    t!("SqlLibrary.new_file_in", path = target_hint.clone()).to_string(),
                    cx,
                    |this, window, cx| this.create_file_dialog(window, cx),
                ))
                .child(header_button(
                    "sql-library-new-folder",
                    IconName::NewFolder,
                    t!("SqlLibrary.new_folder_in", path = target_hint).to_string(),
                    cx,
                    |this, window, cx| this.create_folder_dialog(window, cx),
                ))
                .child(header_button(
                    "sql-library-add-directory",
                    IconName::FolderOpen,
                    t!("Query.add_sql_directory").to_string(),
                    cx,
                    |this, window, cx| this.add_directory(window, cx),
                ))
                .child(header_button(
                    "sql-library-refresh",
                    IconName::Refresh,
                    t!("Common.refresh").to_string(),
                    cx,
                    |this, _window, cx| this.reload(cx),
                )),
        )
        .into_any_element()
}

/// 头部按钮。
fn header_button(
    id: &'static str,
    icon: IconName,
    tooltip: String,
    cx: &mut Context<SqlLibraryPanel>,
    handler: impl Fn(&mut SqlLibraryPanel, &mut Window, &mut Context<SqlLibraryPanel>) + 'static,
) -> AnyElement {
    IconButton::new(id, Icon::new(icon))
        .hit_size(Size::Size(px(24.)))
        .glyph_size(OneIconSize::Small)
        .tooltip(tooltip)
        .on_click(cx.listener(move |this, _event, window, cx| handler(this, window, cx)))
        .into_any_element()
}

/// 一行树节点：目录可展开，文件点击后用 SQL 编辑器打开。
pub(super) fn render_row(
    row: &SqlLibraryRow,
    selected: bool,
    cx: &mut Context<SqlLibraryPanel>,
) -> AnyElement {
    let hover_bg = cx.theme().secondary;
    let selection_bg = cx.theme().sidebar_accent;
    let icon_color = if row.is_dir {
        cx.theme().muted_foreground
    } else {
        cx.theme().foreground
    };
    let path = row.path.clone();
    let path_for_activate = path.clone();
    let is_dir = row.is_dir;
    let name = row.name.clone();

    let mut main = h_flex()
        .id(SharedString::from(format!(
            "sql-library-row-main-{}",
            row.path.display()
        )))
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .when(row.is_dir, |this| {
            this.child(
                Icon::new(if row.expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .with_size(px(12.))
                .text_color(cx.theme().muted_foreground),
            )
        })
        .child(
            Icon::new(if row.is_dir {
                if row.expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                }
            } else {
                IconName::Query
            })
            .with_size(px(14.))
            .text_color(icon_color),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(name.clone()),
        );

    if !row.is_dir {
        main = main.tooltip({
            let path = path.clone();
            move |window, cx| Tooltip::new(path.display().to_string()).build(window, cx)
        });
    }

    let mut container = h_flex()
        .id(SharedString::from(format!(
            "sql-library-row-{}",
            path.display()
        )))
        .w_full()
        .h(px(ROW_HEIGHT))
        .flex_shrink_0()
        .items_center()
        .pr_1()
        .rounded_sm()
        .when(selected, |this| this.bg(selection_bg))
        .when(!selected, |this| this.hover(|style| style.bg(hover_bg)))
        .pl(px(8. + INDENT_STEP * row.depth as f32))
        .child(main.on_click(cx.listener(move |this, _event, _window, cx| {
            if is_dir {
                this.toggle_directory(path_for_activate.clone(), cx);
            } else {
                this.activate_file(path_for_activate.clone(), cx);
            }
        })));

    if selected {
        let rename_path = path.clone();
        let delete_path = path.clone();
        container = container
            .child(row_action(
                format!("sql-library-rename-{}", path.display()),
                IconName::Edit,
                t!("SqlLibrary.rename").to_string(),
                cx,
                move |this, window, cx| this.rename_dialog(rename_path.clone(), window, cx),
            ))
            .child(row_action(
                format!("sql-library-delete-{}", path.display()),
                IconName::Delete,
                t!("Common.delete").to_string(),
                cx,
                move |this, window, cx| this.delete_dialog(delete_path.clone(), window, cx),
            ));

        if row.is_mounted {
            let unmount_path = path.clone();
            container = container.child(row_action(
                format!("sql-library-unmount-{}", path.display()),
                IconName::Minus,
                t!("SqlLibrary.unmount").to_string(),
                cx,
                move |this, window, cx| this.unmount_dialog(unmount_path.clone(), window, cx),
            ));
        }
    }

    container.into_any_element()
}

fn row_action(
    id: String,
    icon: IconName,
    tooltip: String,
    cx: &mut Context<SqlLibraryPanel>,
    handler: impl Fn(&mut SqlLibraryPanel, &mut Window, &mut Context<SqlLibraryPanel>) + 'static,
) -> AnyElement {
    IconButton::new(SharedString::from(id), Icon::new(icon))
        .hit_size(Size::Size(px(ACTION_BUTTON_SIZE)))
        .glyph_size(OneIconSize::Micro)
        .tooltip(tooltip)
        .on_click(cx.listener(move |this, _event, window, cx| handler(this, window, cx)))
        .into_any_element()
}

/// 库为空或加载失败时的占位内容。
pub(super) fn render_placeholder(message: String, cx: &mut Context<SqlLibraryPanel>) -> AnyElement {
    v_flex()
        .w_full()
        .items_center()
        .gap_1()
        .p_4()
        .child(
            Icon::new(IconName::SqlLibrary)
                .with_size(px(24.))
                .text_color(cx.theme().muted_foreground),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(cx.theme().muted_foreground)
                .child(message),
        )
        .into_any_element()
}

/// 路径简称，用于提示文案。
pub(super) fn short_path(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
