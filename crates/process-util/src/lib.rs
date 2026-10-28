mod program;

pub use program::{
    build_command, child_path_with_program_dir, child_path_with_program_dir_and_path,
    resolve_program,
};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 在独立线程里拉起一个「点完就不用管」的后台子进程(GUI 里打开浏览器、文件
/// 管理器、外部编辑器这类)。
///
/// 不能在 UI/主线程直接 `Command::spawn()`：`spawn` 会在**调用线程**里同步
/// `fork` 并在 fork 之后阻塞读取 exec 错误管道，直到子进程 exec 成功才返回。
/// macOS 上 fork 出来的子进程先跑 atfork 处理器(libsystem_notify /
/// libsystem_collections)，一旦和进程内其它线程持有的 notify / objc 锁互等，
/// 子进程就永远走不到 exec，父线程也就永远读不到 EOF —— 表现为 UI 整个卡死、
/// 无日志、子进程 100% CPU 自旋(见 0.22.0「点击在文件管理器中显示后整窗冻结」
/// 事故)。把 `spawn` 挪到独立线程后，即使真的发生 atfork 互等，被挂住的也只是
/// 这个后台线程，窗口仍然可交互。
///
/// 子进程由本线程 `wait` 回收，避免留下僵尸进程；目标程序若长期驻留，本线程
/// 会一直阻塞在 `wait` 上，属预期行为。
pub fn spawn_detached(
    mut command: std::process::Command,
    what: &'static str,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(format!("spawn-detached-{what}"))
        .spawn(move || match command.spawn() {
            Ok(mut child) => {
                if let Err(error) = child.wait() {
                    tracing::warn!(target: "process_util", %error, what, "后台子进程回收失败");
                }
            }
            Err(error) => {
                tracing::warn!(target: "process_util", %error, what, "后台拉起子进程失败");
            }
        })
        .map(|_handle| ())
}

#[must_use]
pub fn should_hide_background_child_console() -> bool {
    cfg!(windows)
}

#[must_use]
pub fn background_child_creation_flags(existing_flags: u32) -> u32 {
    existing_flags | CREATE_NO_WINDOW
}

pub fn configure_background_child(
    command: &mut std::process::Command,
) -> &mut std::process::Command {
    configure_std_command(command);
    command
}

pub fn configure_tokio_background_child(
    command: &mut tokio::process::Command,
) -> &mut tokio::process::Command {
    configure_tokio_command(command);
    command
}

#[cfg(windows)]
fn configure_std_command(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(background_child_creation_flags(0));
}

#[cfg(not(windows))]
fn configure_std_command(_command: &mut std::process::Command) {}

#[cfg(windows)]
fn configure_tokio_command(command: &mut tokio::process::Command) {
    command.creation_flags(background_child_creation_flags(0));
}

#[cfg(not(windows))]
fn configure_tokio_command(_command: &mut tokio::process::Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_child_flags_include_create_no_window() {
        assert_eq!(CREATE_NO_WINDOW, background_child_creation_flags(0));
        assert_eq!(
            CREATE_NO_WINDOW | 0x0000_0200,
            background_child_creation_flags(0x0000_0200)
        );
    }

    #[test]
    fn background_child_console_hiding_is_windows_only() {
        assert_eq!(cfg!(windows), should_hide_background_child_console());
    }

    /// `spawn_detached` 必须先返回（调用线程不阻塞），子进程随后照常跑完。
    ///
    /// 标记文件预先创建为子进程的 stdout：让子进程直接 `echo done` 写进去，
    /// 而不是把路径嵌进 `cmd /C "echo done > \"路径\""` —— std 的 Windows 参数
    /// 转义会把整条命令再包一层引号并转义内层引号，cmd /C 的去引号规则会把
    /// 重定向目标剥成 `\C:\...\` 语法错（CI 实测 "filename, directory name,
    /// or volume label syntax is incorrect"，标记文件永远不出现）。
    #[test]
    fn spawn_detached_runs_child_without_blocking_caller() {
        let marker = std::env::temp_dir().join(format!(
            "process-util-spawn-detached-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        let _ = std::fs::remove_file(&marker);
        let marker_file = std::fs::File::create(&marker).expect("create marker file");
        let marker_len = std::fs::metadata(&marker)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        assert_eq!(0, marker_len, "标记文件必须是空文件，测试前提才成立");

        #[cfg(windows)]
        let mut command = {
            let mut command = std::process::Command::new("cmd");
            command.arg("/C").arg("echo done");
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = std::process::Command::new("/bin/sh");
            command.arg("-c").arg("echo done");
            command
        };
        command.stdout(marker_file);

        spawn_detached(command, "test-marker").expect("thread should start");

        // 调用线程立即回到这里，子进程在后台线程里跑并写完标记文件。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if std::fs::read_to_string(&marker).is_ok_and(|contents| contents.trim() == "done") {
                let _ = std::fs::remove_file(&marker);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&marker);
        panic!("spawn_detached child did not run within 10s");
    }
}
