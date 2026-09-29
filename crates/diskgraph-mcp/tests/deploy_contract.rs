//! Linux deployment contract (P4 task 5.10, specs ST-05 / AI-01): the systemd
//! unit and packaging script are machine-verified so a drifting deployment
//! example fails here, on the development host, instead of on a server.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn deploy_unit() -> String {
    std::fs::read_to_string(repo_root().join("deploy/diskgraph-mcp.service"))
        .expect("deploy/diskgraph-mcp.service must exist")
}

fn package_script() -> String {
    std::fs::read_to_string(repo_root().join("scripts/package-linux.sh"))
        .expect("scripts/package-linux.sh must exist")
}

#[test]
fn the_unit_runs_unprivileged_with_hardening() {
    let unit = deploy_unit();
    for required in [
        "User=diskgraph",
        "Group=diskgraph",
        "NoNewPrivileges=true",
        "ProtectSystem=strict",
        "Restart=on-failure",
    ] {
        assert!(unit.contains(required), "unit must contain {required}");
    }
    // Root is never acceptable for a metadata server.
    assert!(!unit.contains("User=root"), "the unit must not run as root");
}

#[test]
fn the_unit_defaults_to_loopback_and_keeps_sqlite_local() {
    let unit = deploy_unit();
    assert!(
        unit.contains("--host 127.0.0.1"),
        "the shipped default binds loopback only"
    );
    assert!(
        unit.contains("StateDirectory=diskgraph"),
        "state lives under systemd's managed state dir"
    );
    assert!(
        unit.to_lowercase().contains("network share"),
        "the unit documents the local-only SQLite rule (ST-05)"
    );
}

#[test]
fn the_unit_execstart_names_the_shipped_binary_and_profile() {
    let unit = deploy_unit();
    assert!(unit.contains("diskgraph-mcp"), "ExecStart runs the server");
    assert!(unit.contains("--profile read-full"));
    assert!(unit.contains("--transport streamable-http"));
}

#[test]
fn the_packaging_script_verifies_restart_safety_and_local_only() {
    let script = package_script();
    // The smoke section must prove restart-safety and refuse network SQLite.
    assert!(script.contains("persisted index not visible after restart"));
    assert!(script.contains("LOCAL-ONLY"), "the manifest states ST-05");
    assert!(script.contains("SHA256SUMS"), "checksums are recorded");
    assert!(script.contains("diskgraph-mcp.service"), "unit is shipped");
}

#[test]
fn the_packaging_script_runs_on_linux_hosts_not_darwin() {
    let script = package_script();
    // Honest scoping: the script targets Linux (stat -c, /dev/zero) and is
    // meant for the deployment host; macOS cannot produce this evidence.
    assert!(script.contains("uname -s"));
    assert!(script.contains("cargo build --release --locked"));
}
