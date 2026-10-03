//! 真实 CLI 进程验证 impact 的实际 scope 归属和局部读取。
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

fn run(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .arg("--data-dir")
        .arg(data)
        .arg("--json")
        .args(args)
        .output()
        .unwrap()
}

fn success(data: &Path, args: &[&str]) -> Value {
    let output = run(data, args);
    assert!(
        output.status.success(),
        "code={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let root = temp.path().join("project");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='impact'\n").unwrap();
    std::fs::write(root.join("target/artifact"), b"build output").unwrap();
    let scope =
        success(&data, &["scope", "add", "--root", root.to_str().unwrap()])["data"]["scope_id"]
            .as_str()
            .unwrap()
            .to_owned();
    let revision = success(&data, &["index", "--scope", &scope, "--wait"])["data"]["revision_id"]
        .as_str()
        .unwrap()
        .to_owned();
    (temp, data, scope, revision)
}

#[test]
fn impact_refuses_a_scope_hint_that_does_not_own_the_revision() {
    let (temp, data, _scope, revision) = fixture();
    let other = temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let scope =
        success(&data, &["scope", "add", "--root", other.to_str().unwrap()])["data"]["scope_id"]
            .as_str()
            .unwrap()
            .to_owned();
    let output = run(
        &data,
        &[
            "impact",
            "--scope",
            &scope,
            "--revision",
            &revision,
            "--entity",
            "resource-1",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["error"]["code"], "permission_denied");
}

#[test]
fn impact_does_not_decode_an_unrelated_corrupt_relation() {
    let (_temp, data, scope, revision) = fixture();
    let database = rusqlite::Connection::open(data.join("diskgraph.sqlite")).unwrap();
    let id: String = database
        .query_row("SELECT edge_id FROM relations LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(database.execute("UPDATE relations SET edge_json='not JSON', source_entity_id='unrelated-source', target_entity_id='unrelated-target' WHERE edge_id=?1", [&id]).unwrap(), 1);
    drop(database);
    let answer = success(
        &data,
        &[
            "impact",
            "--scope",
            &scope,
            "--revision",
            &revision,
            "--entity",
            "nonexistent-empty-entity",
        ],
    );
    assert_eq!(answer["data"]["entries"], serde_json::json!([]));
    assert_eq!(answer["data"]["complete"], true);
    assert_eq!(answer["data"]["grants_execution"], false);
}
