use crate::{SqliteSnapshotStore, StoreError};
use diskgraph_core::ResourceLocator;

use super::fixtures::graph;
#[test]
fn staging_is_invisible_until_published_and_latest_advances_atomically() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let first = graph("one", 100);
    let second = graph("two", 150);

    // Stage a partial batch for a job that has not published yet.
    store.append_staging_nodes("job-1", &second.nodes).unwrap();
    assert_eq!(store.staging_node_count("job-1").unwrap(), 2);
    // Ordinary queries see nothing from staging...
    assert!(store.snapshot("two").is_err());
    assert_eq!(store.list_snapshots(None, 10, 0).unwrap().len(), 0);
    // ...and an aborted job leaves nothing behind.
    store.clear_staging("job-1").unwrap();
    assert_eq!(store.staging_node_count("job-1").unwrap(), 0);

    // Publish job-2 then job-3; latest follows each publication.
    store
        .publish_revision("job-2", &first, "rev-1", 1_000)
        .unwrap();
    assert_eq!(
        store
            .latest_revision_for_root(&first.snapshot.root)
            .unwrap(),
        Some("rev-1".into())
    );
    store
        .publish_revision("job-3", &second, "rev-2", 2_000)
        .unwrap();
    assert_eq!(
        store
            .latest_revision_for_root(&second.snapshot.root)
            .unwrap(),
        Some("rev-2".into())
    );
    assert_eq!(store.staging_node_count("job-3").unwrap(), 0);
    let loaded = store.load_revision("rev-2").unwrap();
    assert_eq!(loaded, second);
    assert_eq!(store.revision("rev-1").unwrap().snapshot_id, "one");
    assert!(store.revision("missing").is_err());
}

#[test]
fn stale_fencing_staging_is_removed_without_touching_current_or_other_jobs() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let nodes = graph("staged", 10).nodes;
    for namespace in ["job-a:1", "job-a:2", "job-a:3", "job-b:1"] {
        store.append_staging_nodes(namespace, &nodes).unwrap();
    }

    store.clear_stale_job_staging("job-a", 3).unwrap();

    assert_eq!(store.staging_node_count("job-a:1").unwrap(), 0);
    assert_eq!(store.staging_node_count("job-a:2").unwrap(), 0);
    assert_eq!(
        store.staging_node_count("job-a:3").unwrap(),
        nodes.len() as u64
    );
    assert_eq!(
        store.staging_node_count("job-b:1").unwrap(),
        nodes.len() as u64
    );
    let stale_search_rows: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM scan_staging_search WHERE job_id IN ('job-a:1', 'job-a:2')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale_search_rows, 0);
}

#[test]
fn snapshot_removal_respects_pins_and_revision_dependencies() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let pinned = graph("pinned", 10);
    let referenced = graph("referenced", 20);
    let plain = graph("plain", 30);
    store.save(&pinned).unwrap();
    store.save(&plain).unwrap();

    store.pin_snapshot("pinned", true).unwrap();
    assert!(matches!(
        store.remove_snapshot("pinned").unwrap_err(),
        StoreError::RetentionViolation(_)
    ));
    store.pin_snapshot("pinned", false).unwrap();
    store.remove_snapshot("pinned").unwrap();
    assert!(store.snapshot("pinned").is_err());

    // publish_revision creates the snapshot row itself.
    store
        .publish_revision("job-ref", &referenced, "rev-ref", 5)
        .unwrap();
    assert!(matches!(
        store.remove_snapshot("referenced").unwrap_err(),
        StoreError::RetentionViolation(_)
    ));
    store.remove_snapshot("plain").unwrap();
}

#[test]
fn snapshots_list_pages_newest_first_and_can_filter_by_root() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut other_root = graph("other-root", 50);
    other_root.snapshot.root = ResourceLocator::NativePath("/tmp/other".into());
    other_root.nodes[0].locator = ResourceLocator::NativePath("/tmp/other".into());
    store.save(&graph("old", 100)).unwrap();
    store.save(&graph("new", 150)).unwrap();
    store.save(&other_root).unwrap();

    let root = ResourceLocator::NativePath("/tmp/diskgraph-test".into());
    let same_root = store.list_snapshots(Some(&root), 10, 0).unwrap();
    assert_eq!(same_root.len(), 2);
    assert_eq!(same_root[0].id, "new");

    let page = store.list_snapshots(Some(&root), 1, 0).unwrap();
    assert_eq!(page.len(), 1);
    let second_page = store.list_snapshots(Some(&root), 1, 1).unwrap();
    assert_eq!(second_page[0].id, "old");

    assert_eq!(store.list_snapshots(None, 10, 0).unwrap().len(), 3);
}

#[test]
fn staging_search_statement_is_prepared_once_per_batch_with_unicode_rows_intact() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for count in [1, 64, 8192] {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let template = graph("prepare-cost", 1).nodes[1].clone();
        let nodes: Vec<_> = (0..count)
            .map(|index| {
                let mut node = template.clone();
                node.id = index + 1;
                node.name = format!("İΣ文件-{index}");
                node.locator = ResourceLocator::NativePath(format!("/tmp/İΣ文件-{index}"));
                node
            })
            .collect();
        let preparations = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&preparations);
        store
            .connection
            .authorizer(Some(move |context: AuthContext<'_>| {
                // SQLite authorizer 在语句编译时调用；不是按执行行数统计的 update hook。
                if matches!(
                    context.action,
                    AuthAction::Insert {
                        table_name: "scan_staging_search"
                    }
                ) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
                Authorization::Allow
            }))
            .unwrap();
        store
            .append_staging_nodes("prepared-batch", &nodes)
            .unwrap();
        assert_eq!(preparations.load(Ordering::SeqCst), 1, "batch={count}");
        let rows: Vec<(String, String)> = store.connection.prepare(
            "SELECT name_fold,path_fold FROM scan_staging_search WHERE job_id='prepared-batch' ORDER BY node_seq"
        ).unwrap().query_map([], |row| Ok((row.get(0)?,row.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(rows.len(), count as usize);
        for (index, (name, path)) in rows.iter().enumerate() {
            assert_eq!(name, &format!("İΣ文件-{index}").to_lowercase());
            assert_eq!(path, &format!("/tmp/İΣ文件-{index}").to_lowercase());
        }
        assert_eq!(store.staging_node_count("prepared-batch").unwrap(), count);
    }
}
