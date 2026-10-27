use gpui::{App, actions};

pub(crate) const MARKDOWN_CONTEXT: &str = "NotesMarkdown";

actions!(notes_markdown, [SaveMarkdown, ToggleMarkdownOutline]);

pub(crate) fn init(cx: &mut App) {
    // `secondary-` 别名需要宿主 OS 才能解析，走动态加载路径。
    cx.bind_keys([one_core::keybindings::load_dynamic_keybinding(
        "secondary-s",
        SaveMarkdown,
        Some(MARKDOWN_CONTEXT),
        cx,
    )]);
}
