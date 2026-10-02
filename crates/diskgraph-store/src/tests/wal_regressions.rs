use crate::SqliteSnapshotStore;
use crate::sqlite_snapshot_store::WAL_HEAL_THRESHOLD_BYTES;
use rusqlite::Connection;

use super::fixtures::{graph, wal_len};
#[test]
fn the_locator_index_is_not_rebuilt() {
    let dir = std::env::temp_dir().join(format!("diskgraph-noindex-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diskgraph.sqlite");
    {
        let mut store = SqliteSnapshotStore::open(&path).unwrap();
        store
            .publish_revision("job", &graph("slim", 100), "rev-0", 1_700_000_000)
            .unwrap();
    }
    // Reopened, so the open-time drop runs against a populated database.
    let store = SqliteSnapshotStore::open(&path).unwrap();
    let count = |name: &str| -> i64 {
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name = ?1",
                [name],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        count("nodes_by_locator"),
        0,
        "the index cost a quarter kilobyte a node and served no read path"
    );
    // The parent index is what queries actually use, and it stays.
    assert_eq!(count("nodes_by_parent_size"), 1);
    assert_eq!(store.children("slim", 1, 0, 10).unwrap().len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_oversized_wal_is_folded_back_into_the_database_at_open() {
    let dir = std::env::temp_dir().join(format!("diskgraph-wal-heal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diskgraph.sqlite");
    {
        let mut store = SqliteSnapshotStore::open(&path).unwrap();
        store
            .publish_revision("job", &graph("one", 100), "rev-0", 1_700_000_000)
            .unwrap();
    }
    // A run that is killed before its log is folded back: the log is a
    // genuine one from a genuine write, left behind because the process
    // never got to the end of its scan.
    let killed = Connection::open(&path).unwrap();
    killed.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    killed
        .execute_batch("CREATE TABLE IF NOT EXISTS fill (payload BLOB);")
        .unwrap();
    for _ in 0..4_000 {
        killed
            .execute("INSERT INTO fill VALUES (zeroblob(1024))", [])
            .unwrap();
    }
    let before = wal_len(&path);
    assert!(
        before > 1 << 20,
        "the killed run must leave megabytes behind, not {before} bytes"
    );
    // Leaked rather than dropped: a closing connection checkpoints and
    // deletes the log, which is the case the heal exists for the *other*
    // side of. A SIGKILL is what leaves the file on disk.
    std::mem::forget(killed);

    // A threshold of one byte is what makes the heal observable: the
    // production threshold is far above any healthy log.
    let connection = Connection::open(&path).unwrap();
    SqliteSnapshotStore::heal_oversized_wal(&path, &connection, 1).unwrap();
    let after = wal_len(&path);
    assert!(after < before, "the log must shrink, not just be checked");
    assert!(after <= WAL_HEAL_THRESHOLD_BYTES as u64);
    // The data the log carried is still in the database: healing is
    // housekeeping, never a rollback.
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM fill", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 4_000);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_healthy_log_is_left_alone() {
    let dir = std::env::temp_dir().join(format!("diskgraph-wal-quiet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diskgraph.sqlite");
    {
        let mut store = SqliteSnapshotStore::open(&path).unwrap();
        store
            .publish_revision("job", &graph("only", 100), "rev-0", 1_700_000_000)
            .unwrap();
    }
    let connection = Connection::open(&path).unwrap();
    // Below the threshold nothing is touched: a checkpoint would rewrite
    // pages for no reason on every open.
    let before = wal_len(&path);
    SqliteSnapshotStore::heal_oversized_wal(&path, &connection, i64::MAX).unwrap();
    assert_eq!(wal_len(&path), before);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_publish_leaves_no_write_ahead_log_behind() {
    let dir = std::env::temp_dir().join(format!("diskgraph-publish-wal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diskgraph.sqlite");
    let mut store = SqliteSnapshotStore::open(&path).unwrap();
    store
        .publish_revision("job", &graph("one", 100), "rev-0", 1_700_000_000)
        .unwrap();
    // The whole log of the scan is redundant the moment the transaction
    // commits; leaving it on disk is what made a four-million-node scan
    // cost nearly twice its snapshot size.
    assert_eq!(
        wal_len(&path),
        0,
        "publishing must fold its log back into the database"
    );
    // And the snapshot is still readable: the checkpoint is housekeeping,
    // not a second commit.
    assert_eq!(store.load("one").unwrap().nodes.len(), 2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_wal_is_capped_so_it_cannot_grow_without_bound() {
    let dir = std::env::temp_dir().join(format!("diskgraph-wal-limit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diskgraph.sqlite");
    let store = SqliteSnapshotStore::open(&path).unwrap();
    let limit: i64 = store
        .connection
        .query_row("PRAGMA journal_size_limit", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        limit, WAL_HEAL_THRESHOLD_BYTES,
        "without this the log never shrinks below its high-water mark"
    );
    std::fs::remove_dir_all(&dir).ok();
}
