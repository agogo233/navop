use std::collections::HashMap;

use connection_tunnel::SshTunnelConfig;

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionType, CredentialEntry, CredentialReferenceLocation, RemoteDesktopProtocol,
};

use super::reference_scan_tests::{
    database_connection, insert_credential, port_forwarding_connection, remote_desktop_connection,
    repositories, ssh_connection, tunnel_connection,
};

fn insert_connection(
    repository: &crate::storage::ConnectionRepository,
    mut connection: crate::storage::StoredConnection,
) -> i64 {
    repository
        .insert(&mut connection)
        .expect("insert connection")
}

fn insert_plain_ssh(repository: &crate::storage::ConnectionRepository) -> i64 {
    let mut connection = ssh_connection(i64::MAX);
    let mut params = connection.to_ssh_params().expect("parse SSH fixture");
    params.credential_reference = None;
    params.jump_server = None;
    params.proxy = None;
    connection.params = serde_json::to_string(&params).expect("serialize SSH fixture");
    insert_connection(repository, connection)
}

fn insert_cloud_credential(
    repository: &crate::storage::ConnectionRepository,
    cloud_id: &str,
) -> i64 {
    let mut credential = CredentialEntry::new("Synced");
    credential.cloud_id = Some(cloud_id.to_string());
    repository
        .credential_repository()
        .insert(&mut credential)
        .expect("insert synced credential")
}

fn set_tunnel_enabled(
    connection: &mut crate::storage::StoredConnection,
    kind: &str,
    enabled: bool,
) {
    match kind {
        "database" => {
            let mut params = connection
                .to_db_connection()
                .expect("parse database fixture");
            params
                .extra_params
                .insert("ssh_tunnel_enabled".to_string(), enabled.to_string());
            connection.params = serde_json::to_string(&params).expect("serialize database");
        }
        "redis" => {
            let mut params = connection.to_redis_params().expect("parse Redis fixture");
            params.ssh_tunnel.as_mut().expect("Redis tunnel").enabled = enabled;
            connection.params = serde_json::to_string(&params).expect("serialize Redis");
        }
        "mongodb" => {
            let mut params = connection
                .to_mongodb_params()
                .expect("parse MongoDB fixture");
            params.ssh_tunnel.as_mut().expect("MongoDB tunnel").enabled = enabled;
            connection.params = serde_json::to_string(&params).expect("serialize MongoDB");
        }
        _ => unreachable!("unsupported tunnel fixture"),
    }
}

#[test]
fn disabled_tunnels_do_not_create_credential_references() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let ssh_id = insert_plain_ssh(&repository);

    for kind in ["database", "redis", "mongodb"] {
        let mut connection = tunnel_connection(kind, ssh_id);
        set_tunnel_enabled(&mut connection, kind, false);
        insert_connection(&repository, connection);
    }

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan disabled tunnels");
    assert!(hits.is_empty());
}

#[test]
fn enabled_tunnels_without_connection_ids_fail_closed() {
    for kind in ["database", "redis", "mongodb"] {
        let (_temp, _connection, repository) = repositories();
        let credential_id = insert_credential(&repository);
        let mut connection = tunnel_connection(kind, 42);
        match kind {
            "database" => {
                let mut params = connection.to_db_connection().expect("parse database");
                params.extra_params.remove("ssh_connection_id");
                connection.params = serde_json::to_string(&params).expect("serialize database");
            }
            "redis" => {
                let mut params = connection.to_redis_params().expect("parse Redis");
                params.ssh_tunnel = Some(SshTunnelConfig {
                    enabled: true,
                    connection_id: None,
                    ..Default::default()
                });
                connection.params = serde_json::to_string(&params).expect("serialize Redis");
            }
            "mongodb" => {
                let mut params = connection.to_mongodb_params().expect("parse MongoDB");
                params.ssh_tunnel = Some(SshTunnelConfig {
                    enabled: true,
                    connection_id: None,
                    ..Default::default()
                });
                connection.params = serde_json::to_string(&params).expect("serialize MongoDB");
            }
            _ => unreachable!(),
        }
        insert_connection(&repository, connection);

        let error = repository
            .credential_repository()
            .referencing_connections(credential_id)
            .expect_err("missing tunnel ID must fail closed");
        assert!(
            error.to_string().contains("has no connection ID"),
            "{error}"
        );
    }
}

#[test]
fn invalid_database_tunnel_id_fails_closed() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = tunnel_connection("database", 42);
    let mut params = connection.to_db_connection().expect("parse database");
    params
        .extra_params
        .insert("ssh_connection_id".to_string(), "not-an-id".to_string());
    connection.params = serde_json::to_string(&params).expect("serialize database");
    insert_connection(&repository, connection);

    let error = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect_err("invalid database tunnel ID must fail closed");
    assert!(
        error
            .to_string()
            .contains("Invalid database SSH connection ID not-an-id"),
        "{error}"
    );
}

#[test]
fn missing_and_non_ssh_tunnel_targets_fail_closed() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    insert_connection(&repository, port_forwarding_connection(999_999));
    let error = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect_err("missing SSH target must fail closed");
    assert!(error.to_string().contains("was not found"), "{error}");

    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let database_id = insert_connection(&repository, database_connection(i64::MAX));
    insert_connection(&repository, port_forwarding_connection(database_id));
    let error = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect_err("non-SSH target must fail closed");
    assert!(
        error.to_string().contains("is not an SSH connection"),
        "{error}"
    );
}

#[test]
fn vnc_primary_and_proxy_references_are_scanned() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = remote_desktop_connection(credential_id);
    let mut params = connection
        .to_remote_desktop_params()
        .expect("parse remote desktop fixture");
    params.protocol = RemoteDesktopProtocol::Vnc;
    connection.connection_type = ConnectionType::Vnc;
    connection.params = serde_json::to_string(&params).expect("serialize VNC");
    insert_connection(&repository, connection);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan VNC references");
    assert_eq!(2, hits.len());
    assert!(hits.iter().any(|hit| {
        hit.connection_type == ConnectionType::Vnc
            && hit.location == CredentialReferenceLocation::Primary
    }));
    assert!(hits.iter().any(|hit| {
        hit.connection_type == ConnectionType::Vnc
            && hit.location == CredentialReferenceLocation::Proxy
    }));
}

#[test]
fn unrelated_json_numbers_do_not_create_references() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = database_connection(i64::MAX);
    let mut params = connection.to_db_connection().expect("parse database");
    params.credential_reference = None;
    params.proxy = None;
    params.extra_params = HashMap::from([(
        "unrelated_numeric_value".to_string(),
        credential_id.to_string(),
    )]);
    connection.params = serde_json::to_string(&params).expect("serialize database");
    insert_connection(&repository, connection);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan unrelated number");
    assert!(hits.is_empty());
}

#[test]
fn malformed_serial_params_are_ignored() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = database_connection(i64::MAX);
    connection.connection_type = ConnectionType::Serial;
    connection.params = "{".to_string();
    insert_connection(&repository, connection);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("serial does not support credential references");
    assert!(hits.is_empty());
}

#[test]
fn scanning_summaries_does_not_require_an_unlocked_vault() {
    let (_temp, credential_id, repository) = {
        let (temp, _connection, repository) = repositories();
        let credential_id = super::with_master_key(|| {
            let mut credential = CredentialEntry::new("Locked");
            credential.password = Some("secret".to_string());
            repository
                .credential_repository()
                .insert(&mut credential)
                .expect("insert encrypted credential")
        });
        (temp, credential_id, repository)
    };
    insert_connection(&repository, ssh_connection(credential_id));

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan while vault is locked");
    assert_eq!(3, hits.len());
}

#[test]
fn reference_scanner_matches_stable_cloud_id_when_local_ids_differ() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_cloud_credential(&repository, "credential-cloud-stable");
    let foreign_local_id = credential_id + 10_000;
    let mut connection = database_connection(foreign_local_id);
    let mut params = connection.to_db_connection().expect("parse database");
    params
        .credential_reference
        .as_mut()
        .expect("primary credential reference")
        .credential_cloud_id = Some("credential-cloud-stable".to_string());
    params.proxy = None;
    connection.params = serde_json::to_string(&params).expect("serialize database");
    insert_connection(&repository, connection);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan stable cloud ID");

    assert_eq!(1, hits.len());
    assert_eq!(CredentialReferenceLocation::Primary, hits[0].location);
}

#[test]
fn reference_scanner_does_not_fallback_when_reference_has_wrong_cloud_id() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_cloud_credential(&repository, "credential-cloud-correct");
    let mut connection = database_connection(credential_id);
    let mut params = connection.to_db_connection().expect("parse database");
    params
        .credential_reference
        .as_mut()
        .expect("primary credential reference")
        .credential_cloud_id = Some("credential-cloud-wrong".to_string());
    params.proxy = None;
    connection.params = serde_json::to_string(&params).expect("serialize database");
    insert_connection(&repository, connection);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan wrong cloud ID");

    assert!(hits.is_empty());
}

#[test]
fn delete_checked_rejects_cross_device_cloud_id_tunnel_reference() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_cloud_credential(&repository, "credential-cloud-tunnel");
    let foreign_local_id = credential_id + 10_000;
    let mut ssh = ssh_connection(foreign_local_id);
    let mut params = ssh.to_ssh_params().expect("parse SSH");
    params
        .credential_reference
        .as_mut()
        .expect("primary reference")
        .credential_cloud_id = Some("credential-cloud-tunnel".to_string());
    params
        .jump_server
        .as_mut()
        .and_then(|jump| jump.credential_reference.as_mut())
        .expect("jump server reference")
        .credential_cloud_id = Some("credential-cloud-tunnel".to_string());
    params
        .proxy
        .as_mut()
        .and_then(|proxy| proxy.credential_reference.as_mut())
        .expect("proxy reference")
        .credential_cloud_id = Some("credential-cloud-tunnel".to_string());
    ssh.params = serde_json::to_string(&params).expect("serialize SSH");
    let ssh_id = insert_connection(&repository, ssh);
    insert_connection(&repository, port_forwarding_connection(ssh_id));

    let outcome = repository
        .credential_repository()
        .delete_checked(credential_id)
        .expect("protected cross-device delete");
    let crate::storage::DeleteCredentialOutcome::Referenced(hits) = outcome else {
        panic!("cloud-ID tunnel reference must protect credential deletion");
    };

    assert_eq!(
        3,
        hits.iter()
            .filter(|hit| hit.connection_type == ConnectionType::PortForwarding)
            .count()
    );
}

fn insert_raw_connection(
    connection: &crate::storage::connection::SqliteConnection,
    name: &str,
    connection_type: &str,
    params: &str,
) -> i64 {
    connection
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO connections (name, connection_type, params, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 1, 1)",
                rusqlite::params![name, connection_type, params],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .expect("insert raw connection")
}

#[test]
fn unrecognized_connection_type_does_not_block_credential_deletion() {
    // 未来版本写入、损坏或跨版本导入的类型不能把「删除任意凭据」变成永久失败。
    let (temp, connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    insert_raw_connection(&connection, "Future", "Extention", "{}");

    let outcome = repository
        .credential_repository()
        .delete_checked(credential_id)
        .expect("unknown type must not fail the scan");
    assert_eq!(
        crate::storage::DeleteCredentialOutcome::Deleted,
        outcome,
        "{temp:?}"
    );
}

#[test]
fn known_connection_types_in_database_do_not_block_unrelated_deletion() {
    // 回归：Ftp 与 Extension 曾从扫描器的类型白名单里漏掉，
    // 库里只要有一条这类连接，删除任何凭据都会报
    // "Unknown connection type ... while scanning credentials"。
    let (_temp, connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    insert_raw_connection(
        &connection,
        "Plain FTP",
        "Ftp",
        r#"{"host":"ftp.example.com","port":21,"username":"u","password":"p"}"#,
    );
    insert_raw_connection(
        &connection,
        "Extension",
        "Extension",
        r#"{"schema_version":1,"extension_id":"com.example.demo","contribution_id":"demo","config":{},"secrets":{}}"#,
    );

    let outcome = repository
        .credential_repository()
        .delete_checked(credential_id)
        .expect("known types must scan cleanly");
    assert_eq!(crate::storage::DeleteCredentialOutcome::Deleted, outcome);
}

#[test]
fn ftp_primary_reference_is_scanned() {
    let (_temp, connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let params = format!(
        r#"{{"host":"ftp.example.com","port":21,"username":"u","password":"p",
            "credential_reference":{{"credential_id":{credential_id},"username":true,"password":true}}}}"#
    );
    let connection_id = insert_raw_connection(&connection, "FTP", "Ftp", &params);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan FTP references");

    assert_eq!(1, hits.len());
    assert_eq!(connection_id, hits[0].connection_id);
    assert_eq!(ConnectionType::Ftp, hits[0].connection_type);
    assert_eq!(CredentialReferenceLocation::Primary, hits[0].location);
}

#[test]
fn extension_nested_and_flat_references_are_scanned() {
    let (_temp, connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let nested = format!(
        r#"{{"schema_version":1,"extension_id":"com.example.demo","contribution_id":"demo",
            "config":{{"auth":{{"credential_reference":{{"credential_id":{credential_id},"username":true,"password":true}}}}}},
            "secrets":{{}}}}"#
    );
    let flat = format!(
        r#"{{"schema_version":1,"extension_id":"com.navop.middleware.mqtt","contribution_id":"mqtt",
            "config":{{"host":"broker","port":1883,
                "credential_reference":{{"credential_id":{credential_id},"username":true,"password":true}}}},
            "secrets":{{}}}}"#
    );
    let nested_id = insert_raw_connection(&connection, "Nested", "Extension", &nested);
    let flat_id = insert_raw_connection(&connection, "Flat", "Extension", &flat);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan Extension references");

    assert_eq!(2, hits.len(), "{hits:?}");
    let ids = hits
        .iter()
        .map(|hit| hit.connection_id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(std::collections::HashSet::from([nested_id, flat_id]), ids);
}

#[test]
fn malformed_extension_params_do_not_block_deletion() {
    // 扩展记录的 schema 不合法时不能让删除重新失败，只跳过它的引用信息。
    let (_temp, connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    insert_raw_connection(&connection, "Broken", "Extension", r#"{"config":[]}"#);

    let outcome = repository
        .credential_repository()
        .delete_checked(credential_id)
        .expect("malformed extension params must not fail the scan");
    assert_eq!(crate::storage::DeleteCredentialOutcome::Deleted, outcome);
}
