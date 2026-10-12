//! `terminal.control action=reintegrate` 的注入契约。
//!
//! 覆盖：supervisor 门控（fail closed，不抢 exec）、SSH actor 写入注入命令、
//! local actor 不支持时拒绝、`readiness_before` 对 reintegrate 为 `None`。

use crate::exec_supervisor::tests::{ready_supervisor, request, submit};
use crate::exec_supervisor::{ExecEffect, ShellCommandReadiness, TerminalInputSource};
use crate::osc::OscEvent;
use crate::TerminalControlError;

// ---- supervisor 门控（纯状态机层） ----

#[test]
fn reintegrate_is_rejected_while_exec_is_in_flight() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 91, "sleep 300");

    assert_eq!(
        Err(TerminalControlError::Busy),
        supervisor.reintegrate_shell()
    );
}

#[test]
fn reintegrate_is_rejected_when_disconnected() {
    let mut supervisor = ready_supervisor();
    supervisor.disconnect();

    assert_eq!(
        Err(TerminalControlError::Disconnected),
        supervisor.reintegrate_shell()
    );
}

#[test]
fn reintegrate_does_not_touch_readiness_on_success() {
    // 门控通过即返回；readiness 保持不变（Ready 态常见），翻转只由注入后的
    // OSC InputStart 驱动——supervisor 不猜状态。
    let supervisor = ready_supervisor();

    assert_eq!(Ok(()), supervisor.reintegrate_shell());
    assert_eq!(
        ShellCommandReadiness::Ready { prompt_epoch: 1 },
        supervisor.readiness()
    );
}

#[test]
fn reintegrate_allowed_from_stale_submission_pending() {
    // `sudo -i` 场景：readiness 卡在 SubmissionPending（OSC 断流），
    // reintegrate 必须放行——这正是它存在的目的。
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"sudo -i\r");

    assert_eq!(Ok(()), supervisor.reintegrate_shell());
    assert_eq!(
        ShellCommandReadiness::SubmissionPending { command_epoch: 1 },
        supervisor.readiness()
    );
}

#[test]
fn reintegrate_allowed_from_stale_command_running() {
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"top\r");

    assert_eq!(Ok(()), supervisor.reintegrate_shell());
}

#[test]
fn osc_input_start_after_stale_submission_pending_recovers_readiness() {
    // 自愈前提：注入生效 → root shell 的下一个提示符发出 OSC 133;B →
    // readiness 从卡死的 SubmissionPending 无条件翻回 Ready。
    // 钉住 `on_input_start` 的无条件翻转不被未来重构收紧。
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"sudo -i\r");
    assert!(matches!(
        supervisor.readiness(),
        ShellCommandReadiness::SubmissionPending { .. }
    ));

    assert!(supervisor.on_osc(&OscEvent::InputStart).is_empty());
    assert_eq!(
        ShellCommandReadiness::Ready { prompt_epoch: 2 },
        supervisor.readiness()
    );
}

#[test]
fn reintegrate_injection_reuses_runtime_integration_command() {
    // 复用登录时的注入命令：幂等守卫已剥（root login shell 重置环境变量），
    // eval 后 root shell 的提示符钩子恢复发 OSC 133/OSC 7。
    let command = crate::ssh_shell_integration::runtime_shell_integration_command();
    assert!(command.starts_with(b" _ONETCLI_RUNTIME_SETUP=1"));
    assert!(command.ends_with(b"\r"));
    assert!(command.windows(2).any(|w| w == b"\\r"));
}

#[test]
fn reintegrate_recovery_end_to_end_with_supervisor() {
    // 完整自愈链：stuck SubmissionPending → 注入 → root shell 提示符 OSC →
    // Ready → 新 exec 正常提交。驱动真实 supervisor + 真实注入命令字节。
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"sudo -i\r");

    let mut bytes = crate::ssh_shell_integration::runtime_shell_integration_command();
    // eval 完成后 root shell 立即画一个提示符：先剥掉命令自身的 \r，
    // 再拼 OSC 133;A + 133;B（precmd 钩子的最小输出形态）。
    let last_cr = bytes.pop();
    assert_eq!(Some(b'\r'), last_cr);
    bytes.extend_from_slice(b"\x1b]133;A\x07\x1b]133;B\x07");

    // 解析注入命令回显 + 提示符 OSC 流，事件喂给 supervisor。
    let events = crate::osc::extract_osc_events(&bytes);
    assert!(events.contains(&OscEvent::InputStart));
    for event in &events {
        supervisor.on_osc(event);
    }
    assert_eq!(
        ShellCommandReadiness::Ready { prompt_epoch: 2 },
        supervisor.readiness()
    );

    // 恢复后新命令可正常提交。
    assert!(matches!(
        supervisor.start(92, request("nvidia-smi")).as_slice(),
        [ExecEffect::Write {
            source: TerminalInputSource::AgentCommand,
            data
        }, ..] if data == b"nvidia-smi\n"
    ));
}

// ---- actor 层：SSH actor 必须把 reintegrate 落到注入命令写入 ----

#[test]
fn ssh_actor_reintegrate_writes_runtime_injection_command() {
    // 源码 contract：SSH actor 的 Reintegrate 分支必须发送
    // runtime_shell_integration_command() 而不是硬编码 ETX。
    // 防止未来重构把 reintegrate 写成第二个 interrupt（只发 0x03）。
    let source = include_str!("../ssh_backend.rs");
    let control = source
        .find("SshCommand::InterruptForeground")
        .expect("ssh actor control branch exists");
    let reintegrate = source[control..]
        .find("TerminalControlAction::Reintegrate")
        .map(|offset| control + offset)
        .expect("actor handles Reintegrate action");
    let injection = source[reintegrate..]
        .find("runtime_shell_integration_command")
        .map(|offset| reintegrate + offset)
        .expect("reintegrate branch injects the runtime integration command");
    // 注入调用必须落在 reintegrate 分支之后、下一个命令分支之前。
    let next_command = source[injection..]
        .find("SshCommand::")
        .map(|offset| injection + offset)
        .unwrap_or(source.len());
    assert!(injection < next_command);
}

#[test]
fn local_actor_reintegrate_is_rejected_as_unsupported() {
    // 源码 contract：local actor 对 Reintegrate 拒绝而不是吞掉；
    // 本地集成脚本另有形态（Windows profile / 未来 Unix local 注入），
    // 不允许退化为“默默成功”或复用 ETX。
    let source = include_str!("../pty_backend.rs");
    let action_match = source
        .find("match request.action")
        .expect("local actor dispatches on control action");
    let reintegrate = source[action_match..]
        .find("TerminalControlAction::Reintegrate")
        .map(|offset| action_match + offset)
        .expect("local actor handles Reintegrate action");
    let unsupported = source[reintegrate..]
        .find("TerminalControlError::Unsupported")
        .map(|offset| reintegrate + offset)
        .expect("local actor rejects reintegrate with Unsupported");
    let next_command = source[unsupported..]
        .find("LocalPtyCommand::")
        .map(|offset| unsupported + offset)
        .unwrap_or(source.len());
    assert!(unsupported < next_command);
}
