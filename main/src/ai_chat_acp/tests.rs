use std::path::PathBuf;

use super::{normalize_acp_agent_config_ids, preset_agent_entries_with, resolve_preset_command};
use ai_chat_view::{AcpAgentConfig, AcpTransport};

use super::user_config::{parse_user_config, resolve_override};

fn fake_resolver(path: &'static str) -> impl Fn(&str) -> Option<PathBuf> {
    move |_command| Some(PathBuf::from(path))
}

#[test]
fn normalizes_duplicate_acp_agent_config_ids() {
    let mut configs = vec![
        AcpAgentConfig::new("agent", "One", "codex-acp"),
        AcpAgentConfig::new("agent", "Two", "claude-agent-acp"),
        AcpAgentConfig::new("", "Three", "opencode"),
    ];

    normalize_acp_agent_config_ids(&mut configs);

    let ids = configs
        .iter()
        .map(|config| config.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(vec!["agent", "agent-2", "acp-agent-3"], ids);
}

#[test]
fn presets_resolve_commands_and_extend_the_child_path() {
    let user_config = parse_user_config(r#"{"version": 2, "agents": {}}"#).unwrap();
    let entries =
        preset_agent_entries_with(&user_config, &|_| None, fake_resolver("/opt/acp/bin/npm"));

    let claude = entries
        .iter()
        .find(|entry| entry.id.as_ref() == "claude-acp")
        .expect("claude preset");
    let config = claude.config.as_ref().expect("resolved config");
    let AcpTransport::Stdio { command, env, .. } = &config.transport;
    assert_eq!("/opt/acp/bin/npm", command);
    let path = env
        .iter()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.as_str())
        .expect("PATH is injected for npm-backed presets");
    assert!(path.starts_with("/opt/acp/bin"), "{path}");
}

#[test]
fn missing_preset_command_is_reported_without_hiding_other_presets() {
    let user_config = parse_user_config(r#"{"version": 2, "agents": {}}"#).unwrap();
    let entries = preset_agent_entries_with(&user_config, &|_| None, |command| {
        (command != "gemini").then(|| PathBuf::from("/opt/acp/bin/npm"))
    });

    let gemini = entries
        .iter()
        .find(|entry| entry.id.as_ref() == "gemini-acp")
        .expect("gemini preset");
    assert!(gemini.config.is_none());
    let diagnostic = gemini.diagnostic.as_ref().expect("diagnostic");
    assert!(diagnostic.message.contains("找不到命令 `gemini`"));
    assert!(
        entries
            .iter()
            .filter(|entry| entry.id.as_ref() != "gemini-acp")
            .all(|entry| entry.config.is_some()),
        "only the missing preset should be invalid"
    );
}

#[test]
fn overrides_apply_to_presets_without_turning_them_into_user_agents() {
    let user_config = parse_user_config(
        r#"{
            "version": 2,
            "agents": {
                "codex-acp": {
                    "args": ["--experimental"],
                    "env": {"LOG_LEVEL": "debug"},
                    "enabled": false
                }
            }
        }"#,
    )
    .unwrap();
    let entries =
        preset_agent_entries_with(&user_config, &|_| None, fake_resolver("/opt/acp/bin/npm"));

    let codex = entries
        .iter()
        .find(|entry| entry.id.as_ref() == "codex-acp")
        .expect("codex preset");
    assert!(!codex.enabled);
    let config = codex.config.as_ref().expect("resolved config");
    let AcpTransport::Stdio { args, env, .. } = &config.transport;
    assert_eq!(&vec!["--experimental".to_string()], args);
    assert!(
        env.iter()
            .any(|(name, value)| name == "LOG_LEVEL" && value == "debug")
    );
}

#[test]
fn explicit_command_paths_must_exist() {
    let mut config = AcpAgentConfig::new("custom", "Custom", "/definitely/missing/acp");
    let error = resolve_preset_command(&mut config, |_| None).unwrap_err();
    assert!(
        error.message.contains("/definitely/missing/acp"),
        "{}",
        error.message
    );

    let command = std::env::current_exe().expect("test executable");
    let mut config = AcpAgentConfig::new("custom", "Custom", command.display().to_string());
    resolve_preset_command(&mut config, |_| None).expect("existing absolute path is accepted");
}

#[test]
fn resolves_environment_reference_without_storing_secret() {
    let parsed = parse_user_config(
        r#"{
            "version": 1,
            "agents": {
                "codex-acp": {
                    "env": {"OPENAI_API_KEY": "${env:OPENAI_API_KEY}"}
                }
            }
        }"#,
    )
    .unwrap();

    let resolved = resolve_override(&parsed.agents["codex-acp"], |name| {
        (name == "OPENAI_API_KEY").then(|| "secret-value".to_string())
    })
    .unwrap();

    assert_eq!(
        Some("secret-value"),
        resolved.env.get("OPENAI_API_KEY").map(String::as_str)
    );
}

#[test]
fn rejects_plaintext_sensitive_value() {
    let error = parse_user_config(
        r#"{
            "version": 1,
            "agents": {
                "codex-acp": {
                    "env": {"OPENAI_API_KEY": "plaintext"}
                }
            }
        }"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("OPENAI_API_KEY must use ${env:NAME}")
    );
}

#[test]
fn one_invalid_override_does_not_hide_other_agents() {
    let user_config = parse_user_config(
        r#"{
            "version": 1,
            "agents": {
                "claude-acp": {
                    "env": {"OPENAI_API_KEY": "${env:MISSING_KEY}"}
                }
            }
        }"#,
    )
    .unwrap();
    let entries =
        preset_agent_entries_with(&user_config, &|_| None, fake_resolver("/opt/acp/bin/npm"));

    let claude = entries
        .iter()
        .find(|entry| entry.id.as_ref() == "claude-acp")
        .expect("claude preset");
    assert!(claude.config.is_none());
    assert!(claude.diagnostic.is_some());
    let codex = entries
        .iter()
        .find(|entry| entry.id.as_ref() == "codex-acp")
        .expect("codex preset");
    assert!(codex.config.is_some());
    assert!(codex.diagnostic.is_none());
}
