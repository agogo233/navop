//! 内置 ACP agent 预设。
//!
//! ACP agent 不再通过 `acp_agent` 扩展下发：常用 agent 直接内置成预设条目，
//! 全部由配置驱动——这里只提供「默认命令 + 默认参数」，用户在设置页或
//! `acp-agents.json` 里覆盖参数、环境变量、鉴权方式、超时与启用状态，
//! 也可以另行添加完全自定义的 agent。
//!
//! 预设命令都按「能在终端里直接跑」验证过（见仓库 PR 描述）：
//! - 原生实现 ACP 的 CLI 直接传子命令（`gemini --experimental-acp`、`kimi acp`）；
//! - 只有 npm 包、没有全局可执行文件的 agent 走 `npm exec`，由启动方在解析
//!   命令时补齐 PATH（`process_util::resolve_program`）；
//! - `npm exec` 需要把参数传给包内的 bin 时必须用
//!   `npm exec --yes --package=<pkg>@<ver> -- <bin> <args>`，`--` 之后直接跟
//!   参数的形式会把参数交给 npm 自己，实测握手失败。

use crate::acp::config::AcpAgentConfig;

/// 一个内置的 ACP agent 预设。
#[derive(Clone, Copy, Debug)]
pub struct BuiltinAcpAgentPreset {
    /// 稳定 id：会话延续、模型缓存、`acp-agents.json` 覆盖都用它做键。
    pub id: &'static str,
    /// 切换器与设置页展示的名字。
    pub name: &'static str,
    /// 默认启动命令（相对 PATH 解析）。
    pub command: &'static str,
    /// 默认参数。
    pub args: &'static [&'static str],
}

impl BuiltinAcpAgentPreset {
    /// 展开成一份可直接启动的配置；用户覆盖在读取配置时再叠加。
    pub fn config(&self) -> AcpAgentConfig {
        AcpAgentConfig::new(self.id, self.name, self.command)
            .with_args(self.args.iter().map(|arg| (*arg).to_string()).collect())
    }
}

/// 内置预设列表。设置页、切换器和启动配置都以它为唯一来源。
pub fn builtin_acp_agent_presets() -> &'static [BuiltinAcpAgentPreset] {
    PRESETS
}

const PRESETS: &[BuiltinAcpAgentPreset] = &[
    BuiltinAcpAgentPreset {
        id: "claude-acp",
        name: "Claude Code",
        command: "npm",
        args: &[
            "exec",
            "--yes",
            "--",
            "@agentclientprotocol/claude-agent-acp@0.95.0",
        ],
    },
    BuiltinAcpAgentPreset {
        id: "codex-acp",
        name: "Codex",
        command: "npm",
        args: &[
            "exec",
            "--yes",
            "--",
            "@agentclientprotocol/codex-acp@2.2.2",
        ],
    },
    BuiltinAcpAgentPreset {
        id: "gemini-acp",
        name: "Gemini CLI",
        command: "gemini",
        args: &["--experimental-acp"],
    },
    BuiltinAcpAgentPreset {
        id: "qwen-acp",
        name: "Qwen Code",
        command: "npm",
        args: &[
            "exec",
            "--yes",
            "--package=@qwen-code/qwen-code@0.26.1",
            "--",
            "qwen",
            "--acp",
        ],
    },
    BuiltinAcpAgentPreset {
        id: "kimi-acp",
        name: "Kimi CLI",
        command: "kimi",
        args: &["acp"],
    },
    BuiltinAcpAgentPreset {
        id: "opencode-acp",
        name: "OpenCode",
        command: "opencode",
        args: &["acp"],
    },
    BuiltinAcpAgentPreset {
        id: "copilot-acp",
        name: "GitHub Copilot CLI",
        command: "npm",
        args: &[
            "exec",
            "--yes",
            "--package=@github/copilot@1.0.103",
            "--",
            "copilot",
            "--acp",
        ],
    },
    BuiltinAcpAgentPreset {
        id: "cursor-acp",
        name: "Cursor CLI",
        command: "agent",
        args: &["acp"],
    },
    BuiltinAcpAgentPreset {
        id: "goose-acp",
        name: "Goose",
        command: "goose",
        args: &["acp"],
    },
];

/// 按 id 找预设。
pub fn builtin_acp_agent_preset(id: &str) -> Option<&'static BuiltinAcpAgentPreset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn preset_ids_and_names_are_unique_and_declare_a_command() {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for preset in builtin_acp_agent_presets() {
            assert!(ids.insert(preset.id), "duplicate preset id {}", preset.id);
            assert!(
                names.insert(preset.name),
                "duplicate preset name {}",
                preset.name
            );
            assert!(!preset.command.trim().is_empty(), "{}", preset.id);
            assert!(
                !preset.args.is_empty() || preset.command != "npm",
                "{}",
                preset.id
            );
        }
    }

    #[test]
    fn presets_expand_to_stdio_configs_keyed_by_id() {
        let preset = builtin_acp_agent_preset("gemini-acp").expect("gemini preset");
        let config = preset.config();

        assert_eq!("gemini-acp", config.id.as_ref());
        assert_eq!("Gemini CLI", config.name.as_ref());
        let crate::acp::AcpTransport::Stdio { command, args, env } = &config.transport;
        assert_eq!("gemini", command);
        assert_eq!(&vec!["--experimental-acp".to_string()], args);
        assert!(env.is_empty());
    }

    #[test]
    fn npm_backed_presets_pin_the_package_explicitly() {
        for id in ["claude-acp", "codex-acp", "qwen-acp", "copilot-acp"] {
            let preset = builtin_acp_agent_preset(id).expect(id);
            assert_eq!("npm", preset.command, "{id}");
            let config = preset.config();
            let crate::acp::AcpTransport::Stdio { command, args, .. } = &config.transport;
            assert_eq!("npm", command);
            assert!(args.contains(&"exec".to_string()), "{id}");
            // 两种写法都固定了包名与版本：`-- pkg@ver` 或 `--package=pkg@ver -- bin`。
            let pinned = args.iter().any(|arg| {
                arg.starts_with("--package=") || (arg.starts_with('@') && arg.contains('@'))
            });
            assert!(pinned, "npm presets must pin the package: {id}");
        }
    }
}
