//! SQL 库侧边栏面板：管理库根目录与 SQL 文件。
//!
//! 库是跨连接的：面板只负责文件管理，点击 `.sql` 文件时通过
//! [`SqlLibraryEvent::OpenSqlFile`] 交给数据库页用 SQL 编辑器打开。

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, AsyncApp, Context, EventEmitter, IntoElement, ParentElement,
    PathPromptOptions, Render, Styled, Window, div, px,
};
use gpui_component::dialog::DialogButtonProps;
use gpui_component::input::{Input, InputState};
use gpui_component::notification::Notification;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme, WindowExt as _, v_flex};
use one_core::storage::{
    add_sql_library_directory, create_sql_library_file, create_sql_library_folder,
    delete_sql_library_entry, mounted_sql_library_directories, remove_sql_library_directory,
    rename_sql_library_entry, sql_library_roots,
};
use rust_i18n::t;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use super::tree::{SqlLibraryTree, read_directory};
use super::view;

/// 面板向外通知的事件。
#[derive(Clone, Debug)]
pub enum SqlLibraryEvent {
    /// 用户点击了库内 SQL 文件，请求用 SQL 编辑器打开。
    OpenSqlFile(PathBuf),
}

/// 需要输入名称才能执行的文件操作。
#[derive(Clone, Debug)]
enum LibraryOperation {
    CreateFile { directory: PathBuf },
    CreateFolder { directory: PathBuf },
    Rename { path: PathBuf },
}

impl LibraryOperation {
    fn run(&self, name: String) -> Pin<Box<dyn Future<Output = anyhow::Result<PathBuf>> + Send>> {
        let operation = self.clone();
        Box::pin(async move {
            match operation {
                Self::CreateFile { directory } => create_sql_library_file(&directory, &name),
                Self::CreateFolder { directory } => create_sql_library_folder(&directory, &name),
                Self::Rename { path } => rename_sql_library_entry(&path, &name),
            }
        })
    }
}

pub struct SqlLibraryPanel {
    tree: SqlLibraryTree,
    selected: Option<PathBuf>,
    error: Option<String>,
}

impl EventEmitter<SqlLibraryEvent> for SqlLibraryPanel {}

impl SqlLibraryPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut panel = Self {
            tree: SqlLibraryTree::default(),
            selected: None,
            error: None,
        };
        panel.reload(cx);
        panel
    }

    /// 重新读取根目录列表与所有已展开目录。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            let roots = sql_library_roots()?;
            let mounted = mounted_sql_library_directories().unwrap_or_default();
            Ok::<_, anyhow::Error>((roots, mounted))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((roots, mounted)) => {
                        this.error = None;
                        this.tree.set_roots(roots, mounted.into_iter().collect());
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                this.load_visible(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 读取根目录与已展开目录的子项。
    fn load_visible(&mut self, cx: &mut Context<Self>) {
        let directories = self.tree.visible_directories();
        self.load_directories(directories, cx);
    }

    fn load_directories(&mut self, directories: Vec<PathBuf>, cx: &mut Context<Self>) {
        if directories.is_empty() {
            return;
        }
        let task = cx.background_spawn(async move {
            let mut loaded = Vec::new();
            let mut failure = None;
            for directory in directories {
                match read_directory(&directory) {
                    Ok(entries) => loaded.push((directory, entries)),
                    Err(error) => failure = failure.or(Some(error)),
                }
            }
            (loaded, failure)
        });
        cx.spawn(async move |this, cx| {
            let (loaded, failure) = task.await;
            this.update(cx, |this, cx| {
                for (directory, entries) in loaded {
                    this.tree.set_children(&directory, entries);
                }
                if failure.is_some() {
                    this.error = failure;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn toggle_directory(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.selected = Some(directory.clone());
        if self.tree.toggle(&directory) {
            self.load_directories(vec![directory], cx);
        }
        cx.notify();
    }

    pub(super) fn activate_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.selected = Some(path.clone());
        cx.emit(SqlLibraryEvent::OpenSqlFile(path));
        cx.notify();
    }

    /// 新建 SQL 文件 / 目录的作用目录：选中目录，或选中文件的父目录。
    fn target_directory(&self) -> Option<PathBuf> {
        match self.selected.as_deref() {
            Some(path) if path.is_dir() => Some(path.to_path_buf()),
            Some(path) => path.parent().map(|parent| parent.to_path_buf()),
            None => self.tree.roots().first().cloned(),
        }
    }

    pub(super) fn create_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = self.target_directory() else {
            return;
        };
        self.prompt_for_name(
            t!("SqlLibrary.new_file").to_string(),
            t!("SqlLibrary.file_name").to_string(),
            None,
            LibraryOperation::CreateFile { directory },
            |error| t!("SqlLibrary.create_file_failed", error = error.to_string()).to_string(),
            t!("SqlLibrary.file_created").to_string(),
            Self::after_create_file,
            window,
            cx,
        );
    }

    pub(super) fn create_folder_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = self.target_directory() else {
            return;
        };
        self.prompt_for_name(
            t!("Query.new_folder").to_string(),
            t!("Query.folder_name").to_string(),
            None,
            LibraryOperation::CreateFolder { directory },
            |error| t!("SqlLibrary.create_folder_failed", error = error.to_string()).to_string(),
            t!("Query.query_folder_created").to_string(),
            Self::after_create_folder,
            window,
            cx,
        );
    }

    pub(super) fn rename_dialog(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_dir = path.is_dir();
        self.prompt_for_name(
            if is_dir {
                t!("SqlLibrary.rename_folder").to_string()
            } else {
                t!("SqlLibrary.rename").to_string()
            },
            t!("SqlLibrary.rename_placeholder").to_string(),
            Some(initial),
            LibraryOperation::Rename { path },
            |error| t!("SqlLibrary.rename_failed", error = error.to_string()).to_string(),
            t!("SqlLibrary.renamed").to_string(),
            Self::after_rename,
            window,
            cx,
        );
    }

    pub(super) fn delete_dialog(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = view::short_path(&path);
        let panel = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let panel = panel.clone();
            let target = path.clone();
            dialog
                .overlay(false)
                .title(t!("Common.confirm_delete").to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.delete").to_string())
                        .show_cancel(true),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("SqlLibrary.delete_confirm", name = name.clone()).to_string())
                        .child(t!("Common.irreversible").to_string()),
                )
                .on_ok(move |_, _window, cx| {
                    let target = target.clone();
                    panel.update(cx, |panel, cx| {
                        panel.run_operation(
                            async move {
                                delete_sql_library_entry(&target)?;
                                Ok(target)
                            },
                            t!("SqlLibrary.deleted").to_string(),
                            |error| {
                                t!("SqlLibrary.delete_failed", error = error.to_string())
                                    .to_string()
                            },
                            Self::after_delete,
                            cx,
                        );
                    });
                    true
                })
        });
    }

    pub(super) fn unmount_dialog(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = view::short_path(&root);
        let panel = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let panel = panel.clone();
            let target = root.clone();
            dialog
                .overlay(false)
                .title(t!("SqlLibrary.unmount").to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("SqlLibrary.unmount").to_string())
                        .show_cancel(true),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("SqlLibrary.unmount_confirm", name = name.clone()).to_string()),
                )
                .on_ok(move |_, _window, cx| {
                    let target = target.clone();
                    panel.update(cx, |panel, cx| {
                        panel.run_operation(
                            async move {
                                remove_sql_library_directory(&target)?;
                                Ok(target)
                            },
                            t!("SqlLibrary.unmounted").to_string(),
                            |error| {
                                t!("SqlLibrary.unmount_failed", error = error.to_string())
                                    .to_string()
                            },
                            Self::after_unmount,
                            cx,
                        );
                    });
                    true
                })
        });
    }

    pub(super) fn add_directory(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            multiple: false,
            directories: true,
            prompt: Some(t!("Query.select_sql_directory").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(directory) = paths.into_iter().next() else {
                return;
            };
            let task = cx.background_spawn(async move { add_sql_library_directory(&directory) });
            match task.await {
                Ok(_) => {
                    this.update(cx, |this, cx| {
                        this.selected = None;
                        this.reload(cx);
                    })
                    .ok();
                    notify_success_async(cx, t!("Query.sql_directory_added").to_string());
                }
                Err(error) => notify_error_async(
                    cx,
                    t!("Query.add_sql_directory_failed", error = error.to_string()).to_string(),
                ),
            }
        })
        .detach();
    }

    /// 输入名称的通用弹窗：确认后按 `operation` 描述执行后台文件操作。
    #[allow(clippy::too_many_arguments)]
    fn prompt_for_name(
        &mut self,
        title: String,
        placeholder: String,
        initial: Option<String>,
        operation: LibraryOperation,
        failure: fn(&anyhow::Error) -> String,
        success: String,
        apply: fn(&mut Self, PathBuf, &mut Context<Self>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        if let Some(initial) = initial {
            input.update(cx, |input, cx| input.set_value(initial, window, cx));
        }
        let input_for_focus = input.clone();
        let panel = cx.entity();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input = input.clone();
            let value_input = input.clone();
            let panel = panel.clone();
            let operation = operation.clone();
            let success = success.clone();
            dialog
                .overlay(false)
                .title(title.clone())
                .w(px(380.))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.create").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    let name = value_input.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        push_notification(
                            cx,
                            Notification::error(t!("Query.query_name_empty").to_string()),
                        );
                        return false;
                    }
                    let task = operation.run(name);
                    panel.update(cx, |panel, cx| {
                        panel.run_operation(task, success.clone(), failure, apply, cx);
                    });
                    true
                })
                .child(v_flex().p_4().child(Input::new(&input)))
        });
        input_for_focus.update(cx, |input, cx| input.focus(window, cx));
    }

    /// 执行后台文件操作，成功后回调 `apply` 收尾。
    fn run_operation(
        &mut self,
        operation: impl Future<Output = anyhow::Result<PathBuf>> + Send + 'static,
        success: String,
        failure: fn(&anyhow::Error) -> String,
        apply: fn(&mut Self, PathBuf, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) {
        let task = cx.background_spawn(operation);
        cx.spawn(async move |this, cx| match task.await {
            Ok(path) => {
                this.update(cx, |this, cx| apply(this, path, cx)).ok();
                notify_success_async(cx, success);
            }
            Err(error) => notify_error_async(cx, failure(&error)),
        })
        .detach();
    }

    fn after_create_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.expand_parent(&path);
        self.selected = Some(path.clone());
        self.reload(cx);
        cx.emit(SqlLibraryEvent::OpenSqlFile(path));
    }

    fn after_create_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.expand_parent(&path);
        self.selected = Some(path);
        self.reload(cx);
    }

    fn after_rename(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.expand_parent(&path);
        self.selected = Some(path);
        self.reload(cx);
    }

    fn after_delete(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.selected = path.parent().map(|parent| parent.to_path_buf());
        self.reload(cx);
    }

    fn after_unmount(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(path.as_path()) {
            self.selected = None;
        }
        self.reload(cx);
    }

    fn expand_parent(&mut self, path: &PathBuf) {
        if let Some(parent) = path.parent() {
            self.tree.expand(parent);
        }
    }
}

impl Render for SqlLibraryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.tree.rows();
        let selected = self.selected.clone();
        let target = self.target_directory();
        let placeholder = match self.error.clone() {
            Some(error) => Some(t!("SqlLibrary.load_failed", error = error).to_string()),
            None if rows.is_empty() => Some(t!("SqlLibrary.empty").to_string()),
            None => None,
        };

        v_flex()
            .size_full()
            .min_w_0()
            .bg(cx.theme().background)
            .child(view::render_header(target, cx))
            .child(
                div()
                    .w_full()
                    .h(px(1.))
                    .flex_shrink_0()
                    .bg(cx.theme().border),
            )
            .child(
                div().flex_1().min_h_0().w_full().overflow_hidden().child(
                    v_flex()
                        .size_full()
                        .py_1()
                        .overflow_y_scrollbar()
                        .when_some(placeholder, |this, message| {
                            this.child(view::render_placeholder(message, cx))
                        })
                        .children(rows.iter().map(|row| {
                            let is_selected = selected.as_deref() == Some(row.path.as_path());
                            view::render_row(row, is_selected, cx)
                        })),
                ),
            )
    }
}

/// 在活动窗口里弹通知；没有窗口时静默忽略。
fn push_notification(cx: &mut App, notification: Notification) {
    if let Some(window) = cx.active_window() {
        let _ = window.update(cx, |_, window, cx| {
            window.push_notification(notification.autohide(true), cx);
        });
    }
}

fn notify_success_async(cx: &mut AsyncApp, message: String) {
    cx.update(|cx| push_notification(cx, Notification::success(message)));
}

fn notify_error_async(cx: &mut AsyncApp, message: String) {
    cx.update(|cx| push_notification(cx, Notification::error(message)));
}
