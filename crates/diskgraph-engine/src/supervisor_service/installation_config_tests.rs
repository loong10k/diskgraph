use super::InstallationConfig;

fn original() -> serde_json::Value {
    serde_json::json!({
        "schema_version":1, "frontend_uid":1000, "service_uid":23111, "service_gid":23111,
        "state_dir":"/var/lib/diskgraph-supervisor", "data_dir":"/var/lib/diskgraph-supervisor/data",
        "socket_path":"/run/diskgraph-supervisor/frontend.sock", "cli_path":"/usr/libexec/diskgraph/diskgraph",
        "cli_sha256":"ab".repeat(32), "worker_path":"/usr/libexec/diskgraph/diskgraph-scan-worker",
        "worker_sha256":"cd".repeat(32)
    })
}

#[test]
fn fixed_installer_configuration_decodes_without_creating_paths() {
    let input = original();
    let config = InstallationConfig::decode(&serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(config.service_uid, 23111);
    assert_eq!(config.frontend_uid, 1000);
}

#[test]
fn installation_rejects_identity_protocol_and_path_injection() {
    for (field, value) in [
        ("schema_version", serde_json::json!(2)),
        ("service_uid", serde_json::json!(0)),
        ("service_gid", serde_json::json!(0)),
        ("frontend_uid", serde_json::json!(23111)),
        ("frontend_uid", serde_json::json!(u32::MAX)),
        ("cli_path", serde_json::json!("relative/cli")),
        ("cli_path", serde_json::json!("C:\\trusted\\diskgraph.exe")),
        ("cli_path", serde_json::json!("/trusted\\foreign/cli")),
        (
            "worker_path",
            serde_json::json!("/trusted/../foreign/worker"),
        ),
        ("state_dir", serde_json::json!("/")),
        ("data_dir", serde_json::json!("/foreign/data")),
        (
            "data_dir",
            serde_json::json!("/var/lib/diskgraph-supervisor-foreign/data"),
        ),
        ("socket_path", serde_json::json!("/run//socket")),
        (
            "socket_path",
            serde_json::json!(format!("/run/{}", "x".repeat(120))),
        ),
        ("cli_sha256", serde_json::json!("ab")),
        ("worker_sha256", serde_json::json!("AB".repeat(32))),
    ] {
        let mut input = original();
        input[field] = value;
        assert!(
            InstallationConfig::decode(&serde_json::to_vec(&input).unwrap()).is_err(),
            "accepted unsafe installation field {field}: {input}"
        );
    }
}

#[test]
fn installation_rejects_unknown_duplicate_and_oversized_json() {
    let mut input = original();
    input["command"] = serde_json::json!("shell");
    assert!(InstallationConfig::decode(&serde_json::to_vec(&input).unwrap()).is_err());
    let valid = serde_json::to_string(&original()).unwrap();
    let duplicate = valid.replacen('{', "{\"service_uid\":23111,", 1);
    assert!(InstallationConfig::decode(duplicate.as_bytes()).is_err());
    let oversized = format!("{}{}", valid, " ".repeat(4097));
    assert!(InstallationConfig::decode(oversized.as_bytes()).is_err());
}
