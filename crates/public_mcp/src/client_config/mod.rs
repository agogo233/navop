mod file_io;

use anyhow::{Result, bail};
use file_io::{read_optional_json_object, read_optional_text, write_user_only_file};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const SERVER_NAME: &str = "navop";
const LEGACY_SERVER_NAME: &str = "onetcli";
/// 开发 / 便携运行模式在客户端配置里登记的名字。
///
/// 客户端配置（`~/.claude.json`、`~/.codex/config.toml`、Claude Desktop 配置）
/// 是全局共享的单份文件：开发版和安装版都写 `navop` 时，后启动的那个会把前者
/// 的登记覆盖成自己的发现文件路径，安装版在用的 MCP 客户端就被劫持到开发实例。
/// 开发态改用独立名字与独立托管块，两边各自登记、互不覆盖。
const DEVELOPMENT_SERVER_NAME: &str = "navop-dev";
const PACKAGE_NAME: &str = "@navop/mcp";
const NPX_COMMAND: &str = "npx";
/// npm distribution tag used for the external MCP stdio launcher.
/// The launcher resolves the current published package at startup.
pub const NAVOP_MCP_CLIENT_TAG: &str = "latest";
const CODEX_BEGIN: &str = "# BEGIN NAVOP MCP";
const CODEX_END: &str = "# END NAVOP MCP";
const DEV_CODEX_BEGIN: &str = "# BEGIN NAVOP MCP (DEV)";
const DEV_CODEX_END: &str = "# END NAVOP MCP (DEV)";
const LEGACY_CODEX_BEGIN: &str = "# BEGIN ONETCLI PUBLIC MCP";
const LEGACY_CODEX_END: &str = "# END ONETCLI PUBLIC MCP";

/// 宿主注入的运行模式；默认按正式安装态处理，保持历史行为。
static RUNS_INSTALLED: OnceLock<bool> = OnceLock::new();

/// 注入当前进程是否为正式安装态。只在启动早期调用一次，重复调用返回 `false`。
pub fn set_client_config_installed_mode(installed: bool) -> bool {
    RUNS_INSTALLED.set(installed).is_ok()
}

fn runs_installed() -> bool {
    RUNS_INSTALLED.get().copied().unwrap_or(true)
}

fn server_name_for(installed: bool) -> &'static str {
    if installed {
        SERVER_NAME
    } else {
        DEVELOPMENT_SERVER_NAME
    }
}

fn codex_markers_for(installed: bool) -> (&'static str, &'static str) {
    if installed {
        (CODEX_BEGIN, CODEX_END)
    } else {
        (DEV_CODEX_BEGIN, DEV_CODEX_END)
    }
}

/// 旧品牌 `onetcli` 的登记只由正式版负责清理：开发版删掉它，安装版就失去了
/// 自己的迁移来源。
fn legacy_cleanup_for(installed: bool) -> bool {
    installed
}

fn server_name() -> &'static str {
    server_name_for(runs_installed())
}

fn codex_markers() -> (&'static str, &'static str) {
    codex_markers_for(runs_installed())
}

fn legacy_cleanup_enabled() -> bool {
    legacy_cleanup_for(runs_installed())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpLaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub package_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientConfigInstall {
    pub launcher_path: PathBuf,
    pub discovery_path: PathBuf,
    pub launch_spec: McpLaunchSpec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientConfigHealth {
    UpToDate,
    NotInstalled,
    NeedsMigration,
    NeedsRepair,
    PackageVersionOutdated,
    MissingHelper,
    UnusableHelper,
}

impl ClientConfigInstall {
    pub fn from_current_app() -> Self {
        Self::from_npx_command(
            crate::discovery::public_mcp_discovery_path(),
            NAVOP_MCP_CLIENT_TAG,
        )
    }

    pub fn from_npx_command(
        discovery_path: impl Into<PathBuf>,
        version: impl Into<String>,
    ) -> Self {
        let discovery_path = discovery_path.into();
        let version = version.into();
        let args = vec![
            "-y".to_string(),
            format!("{PACKAGE_NAME}@{version}"),
            "--discovery".to_string(),
            path_string(&discovery_path),
        ];
        Self {
            launch_spec: McpLaunchSpec {
                command: NPX_COMMAND.to_string(),
                args,
                env: BTreeMap::new(),
                package_version: Some(version),
            },
            launcher_path: PathBuf::from(NPX_COMMAND),
            discovery_path,
        }
    }

    pub fn from_helper_path(
        launcher_path: impl Into<PathBuf>,
        discovery_path: impl Into<PathBuf>,
    ) -> Self {
        let launcher_path = launcher_path.into();
        let discovery_path = discovery_path.into();
        Self {
            launch_spec: McpLaunchSpec {
                command: path_string(&launcher_path),
                args: vec!["--discovery".into(), path_string(&discovery_path)],
                env: BTreeMap::new(),
                package_version: None,
            },
            launcher_path,
            discovery_path,
        }
    }
}

pub fn default_helper_install_path() -> PathBuf {
    PathBuf::from(NPX_COMMAND)
}

pub fn helper_install_path_from_data_dir(_data_dir: impl AsRef<Path>) -> PathBuf {
    default_helper_install_path()
}

pub fn codex_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex/config.toml"))
}

pub fn claude_desktop_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("Claude/claude_desktop_config.json"))
}

pub fn claude_code_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".claude.json"))
}

pub fn install_codex_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    let existing = read_optional_text(path)?;
    let cleaned = remove_managed_blocks(&existing);
    let mut next = cleaned.trim_end().to_string();
    if !next.is_empty() {
        next.push_str("\n\n");
    }
    next.push_str(&codex_managed_block(install));
    write_user_only_file(path, next.into_bytes())
}

pub fn install_claude_desktop_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    install_json_mcp_servers_config(path, install)
}

pub fn install_claude_code_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    install_json_mcp_servers_config(path, install)
}

fn install_json_mcp_servers_config(path: &Path, install: &ClientConfigInstall) -> Result<()> {
    let mut root = read_optional_json_object(path)?;
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    let Value::Object(servers) = servers else {
        bail!("Claude config field `mcpServers` must be a JSON object");
    };
    if legacy_cleanup_enabled() {
        servers.remove(LEGACY_SERVER_NAME);
    }
    servers.insert(server_name().into(), expected_server_config(install));
    write_user_only_file(path, serde_json::to_vec_pretty(&Value::Object(root))?)
}

pub fn agent_mcp_config_json(install: &ClientConfigInstall) -> Result<String> {
    let mut servers = serde_json::Map::new();
    servers.insert(server_name().to_string(), expected_server_config(install));
    Ok(serde_json::to_string_pretty(
        &json!({ "mcpServers": Value::Object(servers) }),
    )?)
}

pub fn inspect_codex_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    if let Some(health) = launcher_unavailable_health(install)? {
        return Ok(health);
    }
    let text = read_optional_text(path)?;
    if legacy_cleanup_enabled()
        && (text.contains(LEGACY_CODEX_BEGIN) || text.contains("[mcp_servers.onetcli]"))
    {
        return Ok(ClientConfigHealth::NeedsMigration);
    }
    let (begin, _end) = codex_markers();
    if !text.contains(begin) {
        return Ok(ClientConfigHealth::NotInstalled);
    }
    classify_config_match(text.contains(&codex_managed_block(install)), &text)
}

pub fn inspect_claude_desktop_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    inspect_json_mcp_servers_config(path, install)
}

pub fn inspect_claude_code_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    inspect_json_mcp_servers_config(path, install)
}

fn inspect_json_mcp_servers_config(
    path: &Path,
    install: &ClientConfigInstall,
) -> Result<ClientConfigHealth> {
    if let Some(health) = launcher_unavailable_health(install)? {
        return Ok(health);
    }
    let root = read_optional_json_object(path)?;
    let Some(servers) = root.get("mcpServers").and_then(Value::as_object) else {
        return Ok(ClientConfigHealth::NotInstalled);
    };
    if legacy_cleanup_enabled() && servers.contains_key(LEGACY_SERVER_NAME) {
        return Ok(ClientConfigHealth::NeedsMigration);
    }
    let Some(server) = servers.get(server_name()) else {
        return Ok(ClientConfigHealth::NotInstalled);
    };
    classify_config_match(
        server == &expected_server_config(install),
        &server.to_string(),
    )
}

fn classify_config_match(matches: bool, text: &str) -> Result<ClientConfigHealth> {
    if matches {
        return Ok(ClientConfigHealth::UpToDate);
    }
    if text.contains(PACKAGE_NAME)
        && !text.contains(&format!("{PACKAGE_NAME}@{NAVOP_MCP_CLIENT_TAG}"))
    {
        return Ok(ClientConfigHealth::PackageVersionOutdated);
    }
    Ok(ClientConfigHealth::NeedsRepair)
}

fn codex_managed_block(install: &ClientConfigInstall) -> String {
    let args = install
        .launch_spec
        .args
        .iter()
        .map(|arg| serde_json::to_string(arg).unwrap())
        .collect::<Vec<_>>()
        .join(", ");
    let (begin, end) = codex_markers();
    format!(
        "{begin}\n[mcp_servers.{}]\ncommand = {}\nargs = [{args}]\n{end}\n",
        server_name(),
        serde_json::to_string(&install.launch_spec.command).unwrap()
    )
}

fn expected_server_config(install: &ClientConfigInstall) -> Value {
    json!({ "command": install.launch_spec.command, "args": install.launch_spec.args })
}

fn launcher_unavailable_health(
    install: &ClientConfigInstall,
) -> Result<Option<ClientConfigHealth>> {
    if install.launch_spec.package_version.is_some() {
        return Ok(None);
    }
    helper_unavailable_health(&install.launcher_path)
}

pub fn helper_unavailable_health(path: &Path) -> Result<Option<ClientConfigHealth>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(ClientConfigHealth::MissingHelper));
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Ok(Some(ClientConfigHealth::MissingHelper));
    }
    if !helper_is_executable(&metadata) {
        return Ok(Some(ClientConfigHealth::UnusableHelper));
    }
    Ok(None)
}

pub fn uninstall_codex_config(path: &Path) -> Result<()> {
    let cleaned = remove_managed_blocks(&read_optional_text(path)?)
        .trim()
        .to_string();
    if cleaned.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_user_only_file(path, cleaned.into_bytes())
}

pub fn uninstall_claude_desktop_config(path: &Path) -> Result<()> {
    uninstall_json_mcp_servers_config(path)
}

pub fn uninstall_claude_code_config(path: &Path) -> Result<()> {
    uninstall_json_mcp_servers_config(path)
}

fn uninstall_json_mcp_servers_config(path: &Path) -> Result<()> {
    let mut root = read_optional_json_object(path)?;
    if let Some(Value::Object(servers)) = root.get_mut("mcpServers") {
        servers.remove(server_name());
        if legacy_cleanup_enabled() {
            servers.remove(LEGACY_SERVER_NAME);
        }
        if servers.is_empty() {
            root.remove("mcpServers");
        }
    }
    if root.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_user_only_file(path, serde_json::to_vec_pretty(&Value::Object(root))?)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(unix)]
fn helper_is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn helper_is_executable(_metadata: &fs::Metadata) -> bool {
    true
}

fn remove_managed_blocks(text: &str) -> String {
    let (begin, end) = codex_markers();
    let cleaned = remove_block(text, begin, end);
    if legacy_cleanup_enabled() {
        return remove_block(&cleaned, LEGACY_CODEX_BEGIN, LEGACY_CODEX_END);
    }
    cleaned
}

fn remove_block(text: &str, begin: &str, end: &str) -> String {
    let mut output = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.trim() == begin {
            skipping = true;
            continue;
        }
        if skipping && line.trim() == end {
            skipping = false;
            continue;
        }
        if !skipping {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        CODEX_BEGIN, CODEX_END, DEVELOPMENT_SERVER_NAME, DEV_CODEX_BEGIN, DEV_CODEX_END,
        SERVER_NAME, codex_markers_for, legacy_cleanup_for, server_name_for,
    };

    #[test]
    fn installed_runs_keep_the_historical_client_registration() {
        assert_eq!("navop", server_name_for(true));
        assert_eq!((CODEX_BEGIN, CODEX_END), codex_markers_for(true));
        assert!(
            legacy_cleanup_for(true),
            "正式版必须继续清理旧品牌 onetcli 的登记"
        );
    }

    #[test]
    fn development_runs_never_reuse_the_installed_registration() {
        assert_eq!("navop-dev", server_name_for(false));
        assert_eq!(
            "navop-dev", DEVELOPMENT_SERVER_NAME,
            "开发态名字必须带后缀，避免覆盖安装版"
        );
        assert_eq!((DEV_CODEX_BEGIN, DEV_CODEX_END), codex_markers_for(false));
        assert_ne!(SERVER_NAME, server_name_for(false));
        assert!(
            !legacy_cleanup_for(false),
            "开发版不能删安装版的 onetcli 迁移来源"
        );
    }
}
