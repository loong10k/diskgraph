//! Git 持久入口的真实既有 API 负控；不以缺失 Rust 类型模拟行为失败。
use crate::{ControlStore, JobKind, SqliteSnapshotStore};

#[test]
fn durable_git_kind_has_a_stable_wire_tag() {
    let kind: JobKind = serde_json::from_str("\"git_evidence\"")
        .expect("durable Git evidence jobs need their own kind");
    assert_eq!(serde_json::to_string(&kind).unwrap(), "\"git_evidence\"");
}

#[test]
fn v7_control_upgrade_has_a_consistent_pre_v8_backup_and_typed_input_table() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    {
        let store = ControlStore::open(&path).unwrap();
        store
            .connection
            .pragma_update(None, "user_version", 7)
            .unwrap();
    }
    let upgraded = ControlStore::open(&path).unwrap();
    assert_eq!(
        upgraded
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert!(
        upgraded
            .connection
            .prepare(
                "SELECT job_id,schema_version,input_json,input_sha256 FROM git_evidence_job_inputs"
            )
            .is_ok()
    );
    let backup = rusqlite::Connection::open(
        directory
            .path()
            .join("migration_backups/control.sqlite.pre-v8.bak"),
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
}

#[test]
fn graph_publication_receipts_are_part_of_the_current_schema() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    assert!(
        store
            .connection
            .prepare("SELECT job_id,input_sha256,receipt_json FROM job_publication_receipts")
            .is_ok(),
        "collector publication needs an immutable receipt in the same graph transaction"
    );
}
