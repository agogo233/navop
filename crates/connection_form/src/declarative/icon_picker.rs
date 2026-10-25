//! Icon 字段:与 SSH 连接表单一致的图标选择器(自动 / 内置图标平铺 / 本地图片)。
//!
//! 存储形态由 [`DeclarativeForm::collect`] 产出的 `{"icon": "<id>"}` 或
//! `{"file": "<path>"}` 决定;两者互斥,「自动」即配置里没有这个键。

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AsyncApp, Context, InteractiveElement as _, IntoElement, ParentElement, PathPromptOptions,
    StatefulInteractiveElement as _, Styled, div, img, px,
};
use gpui_component::{ActiveTheme as _, Icon, Sizable as _, form::field, h_flex, tooltip::Tooltip};
use one_assets::IconName;
use one_core::storage::{SSH_ICON_IDS, ssh_os_icon};
use rust_i18n::t;

use super::{DeclarativeForm, DeclarativeFormField, icon_subkey};

impl DeclarativeForm {
    /// 渲染 Icon 字段:自动、内置图标或本地图片。
    pub(super) fn render_icon_field(
        &self,
        field_info: &DeclarativeFormField,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_component::form::Field> {
        let (icon, file) = self.icon_selection(&field_info.id, cx);
        let mut row = h_flex().gap_2().flex_wrap().items_center();
        row = row.child(self.render_icon_tile(
            field_info,
            "",
            icon.is_empty() && file.is_empty(),
            cx,
        ));
        for id in SSH_ICON_IDS {
            row = row.child(self.render_icon_tile(
                field_info,
                id,
                file.is_empty() && icon == *id,
                cx,
            ));
        }
        row = row.child(self.render_custom_icon_tile(field_info, &file, cx));
        vec![
            field()
                .label(field_info.label.clone())
                .items_center()
                .child(
                    row.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("ConnectionForm.icon_hint").to_string()),
                    ),
                ),
        ]
    }

    /// 当前选择:`(内置图标 ID, 本地图片路径)`,至多一个非空。
    fn icon_selection(&self, field_id: &str, cx: &App) -> (String, String) {
        let icon = self.value(&icon_subkey(field_id, "icon"), cx);
        let file = self.value(&icon_subkey(field_id, "file"), cx);
        if !file.trim().is_empty() {
            (String::new(), file)
        } else {
            (icon, String::new())
        }
    }

    fn set_icon_selection(
        &mut self,
        field_id: &str,
        icon: Option<String>,
        file: Option<String>,
        cx: &mut Context<Self>,
    ) {
        for (part, value) in [("icon", icon), ("file", file)] {
            let Some(value_entity) = self.values.get(&icon_subkey(field_id, part)) else {
                continue;
            };
            let next = value.unwrap_or_default();
            value_entity.update(cx, |current, cx| {
                if *current != next {
                    *current = next;
                    cx.notify();
                }
            });
        }
        cx.notify();
    }

    fn icon_tile_id(field_id: &str, icon_id: &str) -> String {
        format!(
            "declarative-icon-{field_id}-{}",
            if icon_id.is_empty() { "auto" } else { icon_id }
        )
    }

    fn render_icon_tile(
        &self,
        field_info: &DeclarativeFormField,
        icon_id: &str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let field_id = field_info.id.clone();
        let icon_string = (!icon_id.is_empty()).then(|| icon_id.to_string());
        let tile_id = Self::icon_tile_id(&field_info.id, icon_id);
        let tile_selector = tile_id.clone();
        let icon = (!icon_id.is_empty()).then(|| ssh_os_icon(Some(icon_id)));
        div()
            .id(gpui::SharedString::from(tile_id.clone()))
            .debug_selector(move || tile_selector.clone())
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .relative()
            .cursor_pointer()
            .when(selected, |tile| {
                tile.border_2()
                    .border_color(cx.theme().list_active_border)
                    .bg(cx.theme().list_active)
            })
            .when(!selected, |tile| {
                tile.border_1()
                    .border_color(cx.theme().border)
                    .hover(|style| style.bg(cx.theme().list_hover))
            })
            .when_some(icon, |tile, icon| {
                tile.child(icon.color().with_size(px(22.0)))
            })
            .when(icon_id.is_empty(), |tile| {
                tile.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("ConnectionForm.icon_auto").to_string()),
                )
            })
            .when(selected, |tile| {
                tile.child(Self::render_icon_selection_indicator(
                    format!("{tile_id}-selected"),
                    cx,
                ))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_icon_selection(&field_id, icon_string.clone(), None, cx);
            }))
    }

    fn render_custom_icon_tile(
        &self,
        field_info: &DeclarativeFormField,
        file: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let field_id = field_info.id.clone();
        let selected = !file.is_empty();
        let tile_id = Self::icon_tile_id(&field_info.id, "local");
        let tile_selector = tile_id.clone();
        // gpui 没有 Icon::file_path，直接用 img 从文件系统渲染用户选择的图标
        let icon = if selected {
            img(file.to_string()).size_5().into_any_element()
        } else {
            Icon::new(IconName::Upload)
                .with_size(px(18.0))
                .into_any_element()
        };
        div()
            .id(gpui::SharedString::from(tile_id.clone()))
            .debug_selector(move || tile_selector.clone())
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .relative()
            .cursor_pointer()
            .when(selected, |tile| {
                tile.border_2()
                    .border_color(cx.theme().list_active_border)
                    .bg(cx.theme().list_active)
            })
            .when(!selected, |tile| {
                tile.border_1()
                    .border_color(cx.theme().border)
                    .hover(|style| style.bg(cx.theme().list_hover))
            })
            .child(icon)
            .when(selected, |tile| {
                tile.child(Self::render_icon_selection_indicator(
                    format!("{tile_id}-selected"),
                    cx,
                ))
            })
            .tooltip(|window, cx| {
                Tooltip::new(t!("ConnectionForm.icon_local").to_string()).build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.prompt_for_custom_icon(&field_id, cx);
            }))
    }

    fn render_icon_selection_indicator(id: String, cx: &mut Context<Self>) -> impl IntoElement {
        let indicator_background = cx.theme().button_primary;
        let indicator_foreground = cx.theme().button_primary_foreground;
        div()
            .id(gpui::SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .absolute()
            .top(px(-5.0))
            .right(px(-5.0))
            .size(px(14.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(indicator_background)
            .border_1()
            .border_color(indicator_foreground)
            .child(
                Icon::new(IconName::Check)
                    .with_size(px(9.0))
                    .text_color(indicator_foreground),
            )
    }

    fn prompt_for_custom_icon(&mut self, field_id: &str, cx: &mut Context<Self>) {
        let field_id = field_id.to_string();
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("ConnectionForm.icon_select_file").to_string().into()),
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path = path.to_string_lossy().into_owned();
            let _ = this.update(cx, |this, cx| {
                this.set_icon_selection(&field_id, None, Some(path), cx);
            });
        })
        .detach();
    }
}
