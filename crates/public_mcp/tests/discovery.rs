use public_mcp::discovery::{
    DiscoveryDocument, PublicMcpMode, legacy_public_mcp_discovery_path_from,
    public_mcp_discovery_path_from, read_discovery, remove_discovery, write_discovery,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[test]
fn discovery_round_trips_and_remove_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let document = DiscoveryDocument::new(
        std::process::id(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        "secret-token".to_string(),
        PublicMcpMode::Temporary,
    );

    write_discovery(&path, &document).unwrap();

    let loaded = read_discovery(&path).unwrap();
    assert_eq!("navop", loaded.app);
    assert_eq!(document.version, loaded.version);
    assert_eq!(document.app, loaded.app);
    assert_eq!(document.pid, loaded.pid);
    assert_eq!(document.host, loaded.host);
    assert_eq!(document.port, loaded.port);
    assert_eq!(document.token, loaded.token);
    assert_eq!(document.mode, loaded.mode);

    remove_discovery(&path).unwrap();
    remove_discovery(&path).unwrap();
    assert!(!path.exists());
}

#[test]
fn discovery_validation_accepts_navop_and_legacy_onetcli_apps() {
    let mut document = DiscoveryDocument::new(
        1,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        "a".repeat(64),
        PublicMcpMode::Persistent,
    );
    document.validate_for_stdio_bridge().unwrap();
    document.app = "onetcli".to_string();
    document.validate_for_stdio_bridge().unwrap();
    document.app = "other".to_string();
    assert!(document.validate_for_stdio_bridge().is_err());
}

#[test]
fn legacy_compatible_discovery_preserves_endpoint_and_token() {
    let document = DiscoveryDocument::new(
        1,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        "a".repeat(64),
        PublicMcpMode::Persistent,
    );
    let legacy = document.legacy_compatible();
    assert_eq!("onetcli", legacy.app);
    assert_eq!(document.host, legacy.host);
    assert_eq!(document.port, legacy.port);
    assert_eq!(document.token, legacy.token);
}

#[test]
fn discovery_paths_prefer_new_navop_brand_and_retain_legacy_path() {
    let root = std::path::Path::new("/tmp/config");
    assert_eq!(
        root.join("navop/public-mcp.json"),
        public_mcp_discovery_path_from(root)
    );
    assert_eq!(
        root.join("onetcli/public-mcp.json"),
        legacy_public_mcp_discovery_path_from(root)
    );
}

#[test]
fn discovery_paths_follow_the_run_mode_data_dir() {
    use public_mcp::discovery::public_mcp_discovery_paths_in;

    // 正式安装态：数据目录是 `~/Library/Application Support/navop`，
    // 推导结果必须与历史位置逐字一致，避免升级后客户端找不到文件。
    let installed = std::path::Path::new("/Users/example/Library/Application Support/navop");
    let (canonical, legacy) = public_mcp_discovery_paths_in(installed);
    assert_eq!(
        installed.join("public-mcp.json"),
        canonical,
        "canonical 发现文件必须落在数据目录内"
    );
    assert_eq!(
        std::path::Path::new("/Users/example/Library/Application Support/onetcli/public-mcp.json"),
        legacy,
        "legacy 发现文件必须保持历史兼容位置"
    );

    // 开发/便携态：数据根不同，两个文件都必须落在自己的根下。
    let dev = std::path::Path::new("/Users/example/.navop-dev/state");
    let (dev_canonical, dev_legacy) = public_mcp_discovery_paths_in(dev);
    assert_eq!(dev.join("public-mcp.json"), dev_canonical);
    assert_eq!(
        std::path::Path::new("/Users/example/.navop-dev/onetcli/public-mcp.json"),
        dev_legacy
    );
    assert_ne!(
        canonical, dev_canonical,
        "开发版与安装版的发现文件绝不能是同一个文件"
    );
}

#[cfg(unix)]
#[test]
fn discovery_file_is_user_only_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let document = DiscoveryDocument::new(
        std::process::id(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        "secret-token".to_string(),
        PublicMcpMode::Persistent,
    );

    write_discovery(&path, &document).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(0o600, mode);
}
