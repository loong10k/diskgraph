//! 宽目录真实 SQLite 工作量与旧快照回填回归；夹具仅使用内存数据库。
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::{SqliteSnapshotStore, tests::graph};

fn wide_store(unknown: bool) -> SqliteSnapshotStore {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&graph("wide", 100)).unwrap();
    let payload = if unknown {
        "{\"size_known\":false}"
    } else {
        ""
    };
    store.connection.execute(
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<200000)
         INSERT INTO nodes(snapshot_id,id,parent_id,locator_key,name,subtree_bytes,node_json,kind,read_error)
         SELECT 'wide',x+2,1,'{}',printf('child-%09d',x),
                CASE WHEN ?1 THEN 1000 ELSE x END,?2,CASE WHEN ?1 THEN NULL ELSE 'file' END,0 FROM n",
        rusqlite::params![unknown, payload],
    ).unwrap();
    // 构造真实 v8 结构，升级必须补齐缓存，不能依赖新 writer 的内存图。
    store
        .connection
        .execute_batch(
            "DROP TRIGGER snapshots_require_count_writer; ALTER TABLE snapshots DROP COLUMN count_schema; DROP TABLE IF EXISTS child_size_prefix;
         DROP TABLE IF EXISTS directory_counts;
         DROP TABLE IF EXISTS snapshot_counts;
         DROP INDEX IF EXISTS nodes_by_known_parent_size;
         PRAGMA user_version=8;",
        )
        .unwrap();
    SqliteSnapshotStore::initialize(store.connection).unwrap()
}

fn count_steps(store: &SqliteSnapshotStore) -> Arc<AtomicU64> {
    let steps = Arc::new(AtomicU64::new(0));
    let observed = steps.clone();
    store
        .connection
        .progress_handler(
            100,
            Some(move || {
                observed.fetch_add(100, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    steps
}

#[test]
fn wide_directory_counts_are_exact_without_walking_siblings() {
    let store = wide_store(false);
    let steps = count_steps(&store);
    assert_eq!(
        store.child_counts("wide", 1, 100000).unwrap(),
        (200001, 100001)
    );
    assert_eq!(store.child_counts("wide", 1, 0).unwrap(), (200001, 200001));
    assert_eq!(store.child_counts("wide", 1, 200001).unwrap(), (200001, 0));
    assert_eq!(store.node_count("wide").unwrap(), 200002);
    assert!(
        steps.load(Ordering::Relaxed) < 1500,
        "exact counts walked wide siblings: {} VM steps",
        steps.load(Ordering::Relaxed)
    );
}

#[test]
fn known_page_does_not_walk_higher_ranked_unknown_siblings() {
    let store = wide_store(true);
    let steps = count_steps(&store);
    let (items, next, unknown) = store.children_page("wide", 1, None, 0, 1).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "cache");
    assert_eq!(next, None);
    assert_eq!(unknown, 200000);
    assert!(
        steps.load(Ordering::Relaxed) < 1500,
        "known page walked unknown siblings: {} VM steps",
        steps.load(Ordering::Relaxed)
    );
}

#[test]
fn all_writer_routes_preserve_unknown_and_threshold_counts() {
    for route in ["save", "trusted", "staging"] {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let mut mixed = graph(route, 100);
        for (id, bytes, known, error) in [
            (3, 50, false, false),
            (4, 100, true, true),
            (5, 0, true, false),
        ] {
            let mut node = mixed.nodes[1].clone();
            node.id = id;
            node.locator =
                diskgraph_core::ResourceLocator::NativePath(format!("/tmp/diskgraph-test/{id}"));
            node.name = id.to_string();
            node.subtree_bytes = bytes;
            node.size_known = known;
            node.read_error = error;
            mixed.nodes.push(node);
        }
        match route {
            "save" => store.save(&mixed).unwrap(),
            "trusted" => store.publish_revision("job", &mixed, "rev", 1).unwrap(),
            _ => {
                store.append_staging_nodes("job", &mixed.nodes).unwrap();
                store
                    .publish_revision_owned("job", &mixed, "rev", 1, Some(("server", "scope")))
                    .unwrap();
            }
        }
        assert_eq!(store.node_count(route).unwrap(), 5);
        assert_eq!(store.child_counts(route, 1, 50).unwrap(), (4, 3));
        assert_eq!(store.child_counts(route, 1, 51).unwrap(), (4, 2));
        assert_eq!(store.child_counts(route, 1, 101).unwrap(), (4, 0));
        assert_eq!(store.child_counts(route, 5, 0).unwrap(), (0, 0));
        let (known, _, unknown) = store.children_page(route, 1, None, 0, 10).unwrap();
        assert_eq!(known.len(), 2);
        assert_eq!(unknown, 2);
        let (unknown, _) = store.unknown_children_page(route, 1, 0, 10).unwrap();
        assert_eq!(
            unknown.iter().map(|node| node.id).collect::<Vec<_>>(),
            vec![4, 3]
        );
    }
}

#[test]
fn snapshot_removal_cascades_all_count_tables() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&graph("removed", 100)).unwrap();
    store.remove_snapshot("removed").unwrap();
    for table in ["snapshot_counts", "directory_counts", "child_size_prefix"] {
        let count: i64 = store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} retained deleted snapshot data");
    }
}

#[test]
fn v8_backup_contains_old_counts_and_failed_migration_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let mut store = SqliteSnapshotStore::open(&path).unwrap();
    store.save(&graph("old", 100)).unwrap();
    store
        .connection
        .execute_batch(
            "DROP TRIGGER snapshots_require_count_writer; ALTER TABLE snapshots DROP COLUMN count_schema; DROP TABLE child_size_prefix; DROP TABLE directory_counts; DROP TABLE snapshot_counts;
         DROP INDEX nodes_by_known_parent_size; PRAGMA user_version=8;",
        )
        .unwrap();
    let (upgraded, backup) =
        SqliteSnapshotStore::open_with_backup(&path, &dir.path().join("backups")).unwrap();
    let backup = rusqlite::Connection::open(backup.unwrap()).unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        8
    );
    assert_eq!(
        backup
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(upgraded.child_counts("old", 1, 100).unwrap(), (1, 1));
    assert_eq!(upgraded.node_count("old").unwrap(), 2);
    drop(upgraded);
    drop(store);

    // 故意与一个保留表名冲突，证明新结构及版本不会部分提交。
    let legacy = wide_store(false);
    legacy
        .connection
        .execute_batch(
            "DROP TRIGGER snapshots_require_count_writer; ALTER TABLE snapshots DROP COLUMN count_schema; DROP TABLE child_size_prefix; DROP TABLE directory_counts; DROP TABLE snapshot_counts;
         DROP INDEX nodes_by_known_parent_size; PRAGMA user_version=8;
         CREATE TABLE directory_counts(unrelated TEXT);",
        )
        .unwrap();
    assert!(crate::directory_aggregates::migrate(&legacy.connection).is_err());
    assert_eq!(
        legacy
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        8
    );
    assert_eq!(
        legacy
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='snapshot_counts'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        legacy
            .connection
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        200002
    );
}

#[test]
fn old_open_writer_cannot_publish_after_schema_upgrade() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let mut store = SqliteSnapshotStore::open(&path).unwrap();
    store.save(&graph("current", 100)).unwrap();
    store.connection.execute_batch(
        "DROP TRIGGER snapshots_require_count_writer; ALTER TABLE snapshots DROP COLUMN count_schema;
         DROP TABLE child_size_prefix; DROP TABLE directory_counts; DROP TABLE snapshot_counts;
         DROP INDEX nodes_by_known_parent_size; PRAGMA user_version=8;"
    ).unwrap();
    let old_writer = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        old_writer
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        8
    );
    let (store, _) =
        SqliteSnapshotStore::open_with_backup(&path, &directory.path().join("backups")).unwrap();
    // v8 的 INSERT 显式列集合没有新发布格式标记，旧连接不能绕过门禁。
    let published = old_writer.execute(
        "INSERT INTO snapshots(id,root_key,captured_at_unix_ms,snapshot_json,pinned)
         SELECT 'late-v8',root_key,captured_at_unix_ms,snapshot_json,pinned FROM snapshots WHERE id='current'",
        [],
    );
    assert!(
        published.is_err(),
        "old writer created a snapshot without count indexes"
    );
    assert!(store.snapshot("late-v8").is_err());
    assert_eq!(store.node_count("current").unwrap(), 2);
}

#[test]
fn missing_count_indexes_are_an_error_instead_of_confirmed_zero() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&graph("broken", 100)).unwrap();
    store
        .connection
        .execute("DELETE FROM snapshot_counts WHERE snapshot_id='broken'", [])
        .unwrap();
    assert!(store.node_count("broken").is_err());
    assert!(store.child_counts("broken", 1, 0).is_err());
    assert!(store.children_page("broken", 1, None, 0, 1).is_err());
    assert!(store.root_node("broken").is_err());
}

#[test]
fn initialized_schema_does_not_acquire_the_writer_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let store = SqliteSnapshotStore::open(&path).unwrap();
    store.connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let reader = rusqlite::Connection::open(&path).unwrap();
    reader.busy_timeout(std::time::Duration::ZERO).unwrap();
    let opened = SqliteSnapshotStore::initialize(reader);
    store.connection.execute_batch("ROLLBACK;").unwrap();
    assert!(
        opened.is_ok(),
        "unchanged schema required writer lock: {:?}",
        opened.err()
    );
}
