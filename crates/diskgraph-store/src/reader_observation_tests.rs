//! 预开连接的新鲜观察与路径替换边界实验；不改变生产终检连接策略。
use crate::SqliteSnapshotStore;
use std::time::{Duration, Instant};

#[test]
fn preopened_reader_observes_independent_commits_without_a_read_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let mut writer = SqliteSnapshotStore::open(&path).unwrap();
    writer
        .publish_revision(
            "fixture-job",
            &crate::tests::graph("fixture-snapshot", 100),
            "fixture",
            1,
        )
        .unwrap();
    writer
        .connection
        .execute(
            "INSERT INTO revision_ownership VALUES ('fixture','server','scope')",
            [],
        )
        .unwrap();
    let reader = SqliteSnapshotStore::open_reader_until(
        &path,
        Instant::now() + Duration::from_secs(5),
        None,
    )
    .unwrap();
    assert!(reader.connection.is_autocommit());
    assert!(
        reader
            .revision_ownership_matches("fixture", "server", "scope")
            .unwrap()
    );
    writer
        .connection
        .execute(
            "UPDATE revision_ownership SET scope_id='isolated' WHERE revision_id='fixture'",
            [],
        )
        .unwrap();
    assert!(reader.connection.is_autocommit());
    assert!(
        !reader
            .revision_ownership_matches("fixture", "server", "scope")
            .unwrap()
    );
    assert!(
        reader
            .revision_ownership_matches("fixture", "server", "isolated")
            .unwrap()
    );
}

#[cfg(unix)]
#[test]
fn preopened_reader_is_not_proof_of_current_database_path_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let mut writer = SqliteSnapshotStore::open(&path).unwrap();
    writer
        .publish_revision(
            "fixture-job",
            &crate::tests::graph("fixture-snapshot", 100),
            "fixture",
            1,
        )
        .unwrap();
    // 此实验用DELETE journal隔离文件替换；不改变生产WAL配置。
    writer.connection.execute_batch("PRAGMA journal_mode=DELETE; INSERT INTO revision_ownership VALUES ('fixture','server','scope');").unwrap();
    drop(writer);
    let reader = SqliteSnapshotStore::open_reader_until(
        &path,
        Instant::now() + Duration::from_secs(5),
        None,
    )
    .unwrap();
    assert!(
        reader
            .revision_ownership_matches("fixture", "server", "scope")
            .unwrap()
    );
    std::fs::rename(&path, dir.path().join("old.sqlite")).unwrap();
    let mut replacement = SqliteSnapshotStore::open(&path).unwrap();
    replacement
        .connection
        .execute_batch("PRAGMA journal_mode=DELETE;")
        .unwrap();
    replacement
        .publish_revision(
            "replacement-job",
            &crate::tests::graph("replacement-snapshot", 100),
            "fixture",
            1,
        )
        .unwrap();
    replacement
        .connection
        .execute(
            "INSERT INTO revision_ownership VALUES ('fixture','server','isolated')",
            [],
        )
        .unwrap();
    // 旧连接仍指向旧对象；未验证文件身份前，禁止把预先打开的连接当作新鲜路径证据。
    assert!(
        reader
            .revision_ownership_matches("fixture", "server", "scope")
            .unwrap()
    );
    let fresh = SqliteSnapshotStore::open_reader_until(
        &path,
        Instant::now() + Duration::from_secs(5),
        None,
    )
    .unwrap();
    assert!(
        !fresh
            .revision_ownership_matches("fixture", "server", "scope")
            .unwrap()
    );
}
