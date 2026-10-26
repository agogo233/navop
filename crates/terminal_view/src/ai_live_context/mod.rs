//! 终端实时状态 → AI 资源作用域。
//!
//! 侧边栏内置 AI 每轮发送前都会重建资源上下文（见 `ai_chat_view` 的
//! `ResourceContextRefresher`），这里负责把终端「此刻的现场」投影成资源作用域：
//! 当前目录、是否运行全屏程序、选中了哪些内容、窗口尺寸、进程是否已退出。
//!
//! 状态只以作用域形式呈现，模型无需调用工具即可感知；完整屏幕内容仍然通过
//! `terminal.read` 按需拉取，避免每轮把整屏塞进提示词。

mod context;

use agent_runtime::{ResourceRef, ResourceScope};
use alacritty_terminal::term::TermMode;
use terminal::terminal::Terminal;

pub(crate) use context::build_live_resource_context;

/// 作用域键：当前工作目录。
const SCOPE_WORKING_DIR: &str = "cwd";
/// 作用域键：屏幕形态（普通 shell / 全屏交互程序）。
const SCOPE_SCREEN: &str = "screen";
/// 作用域键：终端选中内容摘要。
const SCOPE_SELECTION: &str = "selection";
/// 作用域键：终端窗口尺寸。
const SCOPE_SIZE: &str = "size";
/// 作用域键：终端进程状态。
const SCOPE_PROCESS: &str = "process";

/// 选中内容摘要的最大字符数，避免把整段输出塞进提示词。
const SELECTION_PREVIEW_MAX_CHARS: usize = 120;
/// 摘要里代替换行的占位符，保证摘要始终是单行。
const NEWLINE_PLACEHOLDER: &str = "⏎";

/// 终端此刻的现场，供作用域投影与测试使用。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TerminalLiveState {
    working_dir: Option<String>,
    title: String,
    alternate_screen: bool,
    selection: Option<String>,
    rows: usize,
    cols: usize,
    exited: Option<i32>,
}

impl TerminalLiveState {
    /// 从活动终端抓取一次现场状态。
    pub(crate) fn from_terminal(terminal: &Terminal) -> Self {
        Self {
            working_dir: terminal.current_working_dir().map(str::to_string),
            title: terminal.title().to_string(),
            alternate_screen: terminal.mode().contains(TermMode::ALT_SCREEN),
            selection: terminal.selection_text(),
            rows: terminal.rows(),
            cols: terminal.cols(),
            exited: terminal.child_exited(),
        }
    }

    /// 投影成资源作用域；没有信息的维度直接省略，避免空作用域占提示词。
    pub(crate) fn scopes(&self) -> Vec<ResourceScope> {
        let mut scopes = Vec::new();
        if let Some(dir) = self.working_dir.as_deref().filter(|dir| !dir.is_empty()) {
            scopes.push(ResourceScope::new(
                SCOPE_WORKING_DIR,
                "Working directory",
                dir,
            ));
        }
        scopes.push(ResourceScope::new(
            SCOPE_SCREEN,
            "Screen",
            self.screen_summary(),
        ));
        if let Some(selection) = self
            .selection
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            scopes.push(ResourceScope::new(
                SCOPE_SELECTION,
                "Selection",
                selection_summary(selection),
            ));
        }
        scopes.push(ResourceScope::new(
            SCOPE_SIZE,
            "Size",
            format!("{} rows x {} cols", self.rows, self.cols),
        ));
        if let Some(code) = self.exited {
            scopes.push(ResourceScope::new(
                SCOPE_PROCESS,
                "Process",
                format!("exited with code {code}"),
            ));
        }
        scopes
    }

    fn screen_summary(&self) -> String {
        if !self.alternate_screen {
            return "normal shell".to_string();
        }
        let title = self.title.trim();
        if title.is_empty() {
            "full-screen program running (vi/less/htop); keys go to the app".to_string()
        } else {
            format!("full-screen program running ({title}); keys go to the app")
        }
    }
}

/// 抓取终端现场并投影成作用域。
pub(crate) fn live_resource_scopes(terminal: &Terminal) -> Vec<ResourceScope> {
    TerminalLiveState::from_terminal(terminal).scopes()
}

/// 把实时作用域写到资源上，替换上一次的实时作用域。
///
/// 只替换本模块负责的键，宿主自己写入的作用域（如连接信息）保持不变。
pub(crate) fn apply_live_scopes(resource: &mut ResourceRef, scopes: Vec<ResourceScope>) {
    let live_keys = [
        SCOPE_WORKING_DIR,
        SCOPE_SCREEN,
        SCOPE_SELECTION,
        SCOPE_SIZE,
        SCOPE_PROCESS,
    ];
    resource
        .scopes
        .retain(|scope| !live_keys.contains(&scope.key.as_str()));
    for scope in scopes {
        resource.set_scope(scope);
    }
}

fn selection_summary(selection: &str) -> String {
    let lines = selection.lines().count().max(1);
    let chars = selection.chars().count();
    let preview = selection_preview(selection);
    format!("{lines} lines / {chars} chars: {preview}")
}

fn selection_preview(selection: &str) -> String {
    let mut preview = String::new();
    let mut truncated = false;
    for ch in selection.chars() {
        if preview.chars().count() >= SELECTION_PREVIEW_MAX_CHARS {
            truncated = true;
            break;
        }
        if ch == '\n' {
            preview.push_str(NEWLINE_PLACEHOLDER);
        } else if ch.is_whitespace() && ch != ' ' {
            preview.push(' ');
        } else {
            preview.push(ch);
        }
    }
    let preview = preview.trim().to_string();
    if truncated {
        format!("{preview}…")
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TerminalLiveState {
        TerminalLiveState {
            working_dir: Some("/Users/demo/project".to_string()),
            title: String::new(),
            alternate_screen: false,
            selection: None,
            rows: 40,
            cols: 120,
            exited: None,
        }
    }

    fn scope_value(scopes: &[ResourceScope], key: &str) -> Option<String> {
        scopes
            .iter()
            .find(|scope| scope.key == key)
            .map(|scope| scope.value.clone())
    }

    #[test]
    fn normal_shell_reports_directory_size_and_screen() {
        let scopes = state().scopes();

        assert_eq!(
            scope_value(&scopes, SCOPE_WORKING_DIR).as_deref(),
            Some("/Users/demo/project")
        );
        assert_eq!(
            scope_value(&scopes, SCOPE_SCREEN).as_deref(),
            Some("normal shell")
        );
        assert_eq!(
            scope_value(&scopes, SCOPE_SIZE).as_deref(),
            Some("40 rows x 120 cols")
        );
        assert!(scope_value(&scopes, SCOPE_SELECTION).is_none());
        assert!(scope_value(&scopes, SCOPE_PROCESS).is_none());
    }

    #[test]
    fn alternate_screen_reports_title_and_hints_keys_are_consumed() {
        let mut state = state();
        state.alternate_screen = true;
        state.title = "vim: src/main.rs".to_string();

        let screen = scope_value(&state.scopes(), SCOPE_SCREEN).expect("screen scope");
        assert!(screen.contains("vim: src/main.rs"));
        assert!(screen.contains("keys go to the app"));
    }

    #[test]
    fn selection_is_summarized_into_a_single_line() {
        let mut state = state();
        state.selection = Some("fn main() {\n    println!(\"hi\");\n}".to_string());

        let selection = scope_value(&state.scopes(), SCOPE_SELECTION).expect("selection scope");
        assert!(selection.starts_with("3 lines / "));
        assert!(selection.contains(NEWLINE_PLACEHOLDER));
        assert!(!selection.contains('\n'));
    }

    #[test]
    fn long_selection_is_truncated() {
        let mut state = state();
        state.selection = Some("x".repeat(SELECTION_PREVIEW_MAX_CHARS + 50));

        let selection = scope_value(&state.scopes(), SCOPE_SELECTION).expect("selection scope");
        assert!(selection.ends_with('…'));
        assert!(selection.chars().count() < SELECTION_PREVIEW_MAX_CHARS + 40);
    }

    #[test]
    fn exited_process_is_reported() {
        let mut state = state();
        state.exited = Some(1);

        assert_eq!(
            scope_value(&state.scopes(), SCOPE_PROCESS).as_deref(),
            Some("exited with code 1")
        );
    }

    #[test]
    fn apply_live_scopes_replaces_only_live_keys() {
        let mut resource =
            ResourceRef::new("terminal-1", agent_runtime::ResourceKind::Terminal, "demo")
                .with_scope(ResourceScope::new("screen", "Screen", "stale"))
                .with_scope(ResourceScope::new("host", "Host", "10.0.0.1"));

        apply_live_scopes(&mut resource, state().scopes());

        assert_eq!(
            scope_value(&resource.scopes, "host").as_deref(),
            Some("10.0.0.1")
        );
        assert_eq!(
            scope_value(&resource.scopes, SCOPE_SCREEN).as_deref(),
            Some("normal shell")
        );
    }
}
