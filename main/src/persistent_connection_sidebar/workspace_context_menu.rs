use gpui::{Entity, Window};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use one_assets::IconName;
use rust_i18n::t;

use super::PersistentConnectionSidebar;
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

impl PersistentConnectionSidebar {
    pub(super) fn build_workspace_context_menu(
        menu: PopupMenu,
        view: &Entity<Self>,
        workspace_id: i64,
        expanded: bool,
        window: &mut Window,
        cx: &mut gpui::Context<PopupMenu>,
    ) -> PopupMenu {
        let home = view.read(cx).home_page.clone();
        let Some(workspace) = home
            .read(cx)
            .workspaces
            .iter()
            .find(|workspace| workspace.id == Some(workspace_id))
            .cloned()
        else {
            return menu;
        };
        let sort_order = home.read(cx).workspaces.len() as i32;
        let toggle_view = view.clone();
        let child_home = home.clone();
        let new_home = home.clone();
        let rename_home = home.clone();
        let delete_home = home.clone();

        menu.item(new_child_item(child_home, workspace_id, sort_order))
            .item(new_connection_item(new_home, workspace_id))
            .separator()
            .item(
                PopupMenuItem::new(expansion_label(expanded))
                    .icon(expansion_icon(expanded))
                    .on_click(window.listener_for(&toggle_view, move |this, _, _, cx| {
                        this.toggle_workspace_collapsed(workspace_id, cx);
                    })),
            )
            .item(
                PopupMenuItem::new(t!("Workspace.rename").to_string())
                    .icon(IconName::Edit)
                    .on_click(move |_, window, cx| {
                        show_workspace_dialog(
                            rename_home.clone(),
                            WorkspaceDialogConfig {
                                workspace_id: Some(workspace_id),
                                parent_id: workspace.parent_id,
                                initial_name: workspace.name.clone(),
                                initial_sort_order: workspace.sort_order,
                            },
                            window,
                            cx,
                        );
                    }),
            )
            .item(
                PopupMenuItem::new(t!("Workspace.delete").to_string())
                    .icon(IconName::Remove)
                    .on_click(move |_, window, cx| {
                        delete_home.update(cx, |home, cx| {
                            home.delete_workspace(workspace_id, window, cx);
                        });
                    }),
            )
            .separator()
            .item(refresh_item(home))
    }
}

fn new_child_item(
    home: Entity<crate::home_tab::HomePage>,
    workspace_id: i64,
    sort_order: i32,
) -> PopupMenuItem {
    PopupMenuItem::new(t!("Workspace.new_child").to_string())
        .icon(IconName::Plus)
        .on_click(move |_, window, cx| {
            show_workspace_dialog(
                home.clone(),
                WorkspaceDialogConfig {
                    parent_id: Some(workspace_id),
                    initial_sort_order: Some(sort_order),
                    ..Default::default()
                },
                window,
                cx,
            );
        })
}

/// 分组右键菜单里的「新建连接」：把该分组作为目标分组传进向导，
/// 保存后连接直接落在该分组（用户仍可在表单里改）。
fn new_connection_item(
    home: Entity<crate::home_tab::HomePage>,
    workspace_id: i64,
) -> PopupMenuItem {
    PopupMenuItem::new(t!("Home.new_connection").to_string())
        .icon(IconName::Plus)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.show_new_connection_dialog(Some(workspace_id), window, cx);
            });
        })
}

fn refresh_item(home: Entity<crate::home_tab::HomePage>) -> PopupMenuItem {
    PopupMenuItem::new(t!("Home.refresh").to_string())
        .icon(IconName::Refresh)
        .on_click(move |_, _, cx| {
            home.update(cx, |home, cx| home.refresh_local_home_data(cx));
        })
}

fn expansion_label(expanded: bool) -> String {
    if expanded {
        t!("Workspace.collapse")
    } else {
        t!("Workspace.expand")
    }
    .to_string()
}

fn expansion_icon(expanded: bool) -> IconName {
    if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    }
}

#[cfg(test)]
mod new_connection_target_workspace_tests {
    /// 分组右键「新建连接」必须把该分组透传进向导。
    ///
    /// 用户反馈的「新建连接之后就不在之前那个文件夹里了」，起点就在这里：入口不带分组，
    /// 表单只能预选「未分组」。
    #[test]
    fn group_context_menu_forwards_its_workspace_to_the_wizard() {
        let source = include_str!("workspace_context_menu.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        assert!(
            production.contains("show_new_connection_dialog(Some(workspace_id), window, cx)"),
            "分组右键新建连接必须带上目标分组"
        );
    }

    /// 工具栏 / 空列表 / 快捷键等入口没有分组上下文，只能显式传 None。
    #[test]
    fn other_entries_pass_no_target_workspace() {
        for (name, source) in [
            (
                "home_tab/toolbar.rs",
                include_str!("../home_tab/toolbar.rs"),
            ),
            (
                "home_tab/content.rs",
                include_str!("../home_tab/content.rs"),
            ),
            ("navop_app.rs", include_str!("../navop_app.rs")),
        ] {
            assert!(
                source.contains("show_new_connection_dialog(None"),
                "{name} 里的新建连接入口不应凭空指定分组"
            );
        }
    }
}
