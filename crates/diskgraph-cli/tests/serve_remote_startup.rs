//! CLI 转交远程 MCP 之前不得引导本地管理员；来源：SC-01/PF-06。
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[test]
fn rejected_remote_serve_does_not_open_local_stores_or_bootstrap_admin() {
    let directory = tempfile::tempdir().unwrap();
    let cli = PathBuf::from(env!("CARGO_BIN_EXE_diskgraph"));
    let companion = std::env::var_os("DISKGRAPH_CLI_MCP_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            cli.with_file_name(if cfg!(windows) {
                "diskgraph-mcp.exe"
            } else {
                "diskgraph-mcp"
            })
        });
    assert!(
        companion.is_absolute() && companion.is_file(),
        "actual Cargo-built MCP companion is required; build diskgraph-mcp or supply explicit artifact"
    );
    let bundle = directory.path().join("bundle");
    std::fs::create_dir(&bundle).unwrap();
    let installed_cli = bundle.join(cli.file_name().unwrap());
    std::fs::copy(&cli, &installed_cli).unwrap();
    std::fs::copy(
        &companion,
        bundle.join(if cfg!(windows) {
            "diskgraph-mcp.exe"
        } else {
            "diskgraph-mcp"
        }),
    )
    .unwrap();
    let data = directory.path().join("unopened_data");
    let mut command = Command::new(installed_cli);
    command
        .arg("--data-dir")
        .arg(&data)
        .args(["serve", "--transport", "streamable-http"])
        .stdin(Stdio::null());
    for key in [
        "DISKGRAPH_SCAN_WORKER_PATH",
        "DISKGRAPH_SCAN_WORKER_SHA256",
        "DISKGRAPH_SCAN_WORKER_BYTES",
    ] {
        command.env_remove(key);
    }
    let output = command.output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(6),
        "actual unauthenticated remote MCP startup must reject"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("require --auth"));
    assert!(
        !data.exists(),
        "CLI wrapper must not open stores or bootstrap local admin before remote dispatch"
    );
}
