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
//!
//! 条目数量故意放开：切换器只显示启用中的几条，其余留在设置页按需打开，所以
//! 默认只启用 [`DEFAULT_ENABLED_PRESETS`] 里的常用组合，没用到的 CLI 不会在
//! 每次探活时都被拉起来。

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
    /// 默认环境变量：上游要求关掉自动更新之类的开关放这里。
    pub env: &'static [(&'static str, &'static str)],
}

impl BuiltinAcpAgentPreset {
    /// 展开成一份可直接启动的配置；用户覆盖在读取配置时再叠加。
    pub fn config(&self) -> AcpAgentConfig {
        let config = AcpAgentConfig::new(self.id, self.name, self.command)
            .with_args(self.args.iter().map(|arg| (*arg).to_string()).collect());
        if self.env.is_empty() {
            return config;
        }
        config.with_env(
            self.env
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
        )
    }

    /// 是否默认启用。切换器只列启用中的条目，预设再多也不等于把切换器塞满。
    pub fn enabled_by_default(&self) -> bool {
        DEFAULT_ENABLED_PRESETS.contains(&self.id)
    }
}

/// 默认启用的预设：装了就能用的五个，其余在设置页里逐个打开。
///
/// 顺序跟着 [`PRESETS`] 走，这里只做 id 白名单。
const DEFAULT_ENABLED_PRESETS: &[&str] = &[
    "claude-acp",
    "codex-acp",
    "opencode-acp",
    "dsh-acp",
    "pi-acp",
];

/// 内置预设列表。设置页、切换器和启动配置都以它为唯一来源。
pub fn builtin_acp_agent_presets() -> &'static [BuiltinAcpAgentPreset] {
    PRESETS
}

/// 一个「npm exec 包」形式的预设：先安装再跑，不用用户自己装 CLI。
const fn npm_preset(
    id: &'static str,
    name: &'static str,
    args: &'static [&'static str],
) -> BuiltinAcpAgentPreset {
    BuiltinAcpAgentPreset {
        id,
        name,
        command: "npm",
        args,
        env: &[],
    }
}

/// 一个「本机已装 CLI」形式的预设：命令按 PATH 解析。
const fn cli_preset(
    id: &'static str,
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
) -> BuiltinAcpAgentPreset {
    BuiltinAcpAgentPreset {
        id,
        name,
        command,
        args,
        env: &[],
    }
}

const PRESETS: &[BuiltinAcpAgentPreset] = &[
    npm_preset(
        "claude-acp",
        "Claude Code",
        &[
            "exec",
            "--yes",
            "--",
            "@agentclientprotocol/claude-agent-acp@0.95.0",
        ],
    ),
    npm_preset(
        "codex-acp",
        "Codex",
        &[
            "exec",
            "--yes",
            "--",
            "@agentclientprotocol/codex-acp@2.2.2",
        ],
    ),
    cli_preset("opencode-acp", "OpenCode", "opencode", &["acp"]),
    // `dsh <profile>` 启动的是 profile 里的 app，`acp` 档位跑的就是 ACP server。
    cli_preset("dsh-acp", "DeepSeek Harness", "dsh", &["acp"]),
    npm_preset("pi-acp", "Pi", &["exec", "--yes", "--", "pi-acp@0.0.34"]),
    cli_preset(
        "gemini-acp",
        "Gemini CLI",
        "gemini",
        &["--experimental-acp"],
    ),
    npm_preset(
        "qwen-acp",
        "Qwen Code",
        &[
            "exec",
            "--yes",
            "--package=@qwen-code/qwen-code@0.26.1",
            "--",
            "qwen",
            "--acp",
        ],
    ),
    cli_preset("kimi-acp", "Kimi CLI", "kimi", &["acp"]),
    npm_preset(
        "copilot-acp",
        "GitHub Copilot CLI",
        &[
            "exec",
            "--yes",
            "--package=@github/copilot@1.0.103",
            "--",
            "copilot",
            "--acp",
        ],
    ),
    cli_preset("cursor-acp", "Cursor CLI", "agent", &["acp"]),
    cli_preset("goose-acp", "Goose", "goose", &["acp"]),
    npm_preset(
        "cline-acp",
        "Cline",
        &["exec", "--yes", "--", "cline@3.0.74", "--acp"],
    ),
    npm_preset(
        "qoder-acp",
        "Qoder CLI",
        &["exec", "--yes", "--", "@qoder-ai/qodercli@0.2.14", "--acp"],
    ),
    npm_preset(
        "codebuddy-acp",
        "Codebuddy Code",
        &[
            "exec",
            "--yes",
            "--",
            "@tencent-ai/codebuddy-code@2.177.1",
            "--acp",
        ],
    ),
    npm_preset(
        "kilo-acp",
        "Kilo",
        &["exec", "--yes", "--", "@kilocode/cli@7.8.18", "acp"],
    ),
    npm_preset(
        "minimax-acp",
        "MiniMax Code",
        &["exec", "--yes", "--", "@minimax-ai/code@0.2.7", "acp"],
    ),
    npm_preset(
        "nova-acp",
        "Nova",
        &["exec", "--yes", "--", "@compass-ai/nova@1.1.60", "acp"],
    ),
    BuiltinAcpAgentPreset {
        env: &[("AUGMENT_DISABLE_AUTO_UPDATE", "1")],
        ..npm_preset(
            "auggie-acp",
            "Auggie CLI",
            &["exec", "--yes", "--", "@augmentcode/auggie@0.36.0", "--acp"],
        )
    },
    npm_preset(
        "autohand-acp",
        "Autohand Code",
        &["exec", "--yes", "--", "@autohandai/autohand-acp@0.2.1"],
    ),
    npm_preset(
        "bob-acp",
        "Bob",
        &["exec", "--yes", "--", "@ibm/bob@2.0.9", "acp"],
    ),
    npm_preset(
        "deepagents-acp",
        "DeepAgents",
        &["exec", "--yes", "--", "deepagents-acp@0.1.7"],
    ),
    npm_preset(
        "dirac-acp",
        "Dirac",
        &["exec", "--yes", "--", "dirac-cli@0.5.24", "--acp"],
    ),
    npm_preset(
        "dimcode-acp",
        "DimCode",
        &["exec", "--yes", "--", "dimcode@0.5.22", "acp"],
    ),
    npm_preset(
        "glm-acp",
        "GLM Agent",
        &["exec", "--yes", "--", "glm-acp-agent@1.16.0"],
    ),
    npm_preset(
        "grok-acp",
        "Grok Build",
        &[
            "exec",
            "--yes",
            "--",
            "@xai-official/grok@1.0.72",
            "agent",
            "stdio",
        ],
    ),
    npm_preset(
        "sigit-acp",
        "siGit Code",
        &["exec", "--yes", "--", "@smbcloud/sigit@1.7.5"],
    ),
    npm_preset(
        "agoragentic-acp",
        "Agoragentic",
        &["exec", "--yes", "--", "agoragentic-mcp@1.3.0", "--acp"],
    ),
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
    fn preset_env_is_carried_into_the_stdio_config() {
        let preset = builtin_acp_agent_preset("auggie-acp").expect("auggie preset");
        let config = preset.config();
        let crate::acp::AcpTransport::Stdio { env, .. } = &config.transport;
        assert!(env.contains(&("AUGMENT_DISABLE_AUTO_UPDATE".to_string(), "1".to_string())));
    }

    #[test]
    fn npm_backed_presets_pin_the_package_explicitly() {
        for preset in builtin_acp_agent_presets()
            .iter()
            .filter(|preset| preset.command == "npm")
        {
            let id = preset.id;
            let config = preset.config();
            let crate::acp::AcpTransport::Stdio { command, args, .. } = &config.transport;
            assert_eq!("npm", command);
            assert!(args.contains(&"exec".to_string()), "{id}");
            // 两种写法都固定了包名与版本：`-- pkg@ver` 或 `--package=pkg@ver -- bin`。
            let pinned = args
                .iter()
                .any(|arg| arg.starts_with("--package=") || arg.contains('@'));
            assert!(pinned, "npm presets must pin the package: {id}");
        }
    }

    #[test]
    fn only_the_common_presets_are_enabled_by_default() {
        let enabled = builtin_acp_agent_presets()
            .iter()
            .filter(|preset| preset.enabled_by_default())
            .map(|preset| preset.id)
            .collect::<Vec<_>>();

        assert_eq!(
            vec![
                "claude-acp",
                "codex-acp",
                "opencode-acp",
                "dsh-acp",
                "pi-acp",
            ],
            enabled
        );
    }

    #[test]
    fn dsh_preset_boots_the_acp_profile() {
        let preset = builtin_acp_agent_preset("dsh-acp").expect("dsh preset");
        let config = preset.config();

        assert_eq!("DeepSeek Harness", config.name.as_ref());
        let crate::acp::AcpTransport::Stdio { command, args, .. } = &config.transport;
        assert_eq!("dsh", command);
        assert_eq!(&vec!["acp".to_string()], args);
    }
}
