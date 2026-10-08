//! 同一准入编码写入与整批回滚的真实 SQL 回归。
use crate::{PreparedStagingNode, SqliteSnapshotStore, StoreError};
use diskgraph_core::{QualifiedLocator, ResourceLocator, WindowsObservationGap};

#[test]
fn admitted_encoding_cannot_change_when_source_node_changes() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = crate::tests::graph("prepared", 100);
    let original = graph.nodes[1].clone();
    let locator =
        QualifiedLocator::from_native_path(std::path::Path::new("/tmp/diskgraph-test/cache"))
            .unwrap();
    let prepared = PreparedStagingNode::new(
        &original,
        Some(locator.clone()),
        Some(7),
        None,
        Some(WindowsObservationGap::Unsupported),
    )
    .unwrap();
    graph.nodes[1].name = "changed-after-admission".into();
    graph.nodes[1].locator = ResourceLocator::NativePath("/other/path".into());
    store
        .append_prepared_staging_iter_checked("prepared:1", std::iter::once(&prepared), || Ok(()))
        .unwrap();
    let actual: (String, String, String, Vec<u8>, i64, String) = store.connection.query_row(
        "SELECT s.node_json,f.name_fold,f.path_fold,s.native_locator_raw,s.self_modified_unix_seconds,s.native_observation_gap FROM scan_staging s JOIN scan_staging_search f USING(job_id,node_seq) WHERE s.job_id='prepared:1'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        serde_json::from_str::<diskgraph_core::DiskNode>(&actual.0).unwrap(),
        original
    );
    assert_eq!(actual.1, original.name.to_lowercase());
    assert_eq!(actual.2, locator.display().to_lowercase());
    assert_eq!(actual.3, locator.raw_bytes());
    assert_eq!(actual.4, 7);
    assert_eq!(actual.5, WindowsObservationGap::Unsupported.code());
    let expected = actual.0.len()
        + actual.1.len()
        + actual.2.len()
        + actual.3.len()
        + "native_path".len()
        + locator.encoding().wire_name().len()
        + 8
        + actual.5.len();
    assert_eq!(prepared.encoded_cost(), expected as u64);
}

#[test]
fn prepared_batch_cancel_rolls_back_nodes_and_search_together() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = crate::tests::graph("prepared-cancel", 100);
    let prepared = graph
        .nodes
        .iter()
        .map(|n| PreparedStagingNode::new(n, None, None, None, None).unwrap())
        .collect::<Vec<_>>();
    let mut calls = 0;
    let result = store.append_prepared_staging_iter_checked("cancel:1", prepared.iter(), || {
        calls += 1;
        if calls == 5 {
            return Err(StoreError::Conflict("original cancellation".into()));
        }
        Ok(())
    });
    assert!(
        matches!(result, Err(StoreError::Conflict(ref message)) if message == "original cancellation")
    );
    assert_eq!(store.staging_node_count("cancel:1").unwrap(), 0);
    let search: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM scan_staging_search WHERE job_id='cancel:1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(search, 0);
}

#[test]
fn prepared_commit_cancellation_preserves_the_previous_namespace() {
    use std::cell::Cell;
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = crate::tests::graph("prepared-commit-cancel", 100);
    store
        .append_staging_nodes("previous:1", &graph.nodes)
        .unwrap();
    let prepared = graph
        .nodes
        .iter()
        .map(|node| PreparedStagingNode::new(node, None, None, None, None).unwrap())
        .collect::<Vec<_>>();
    let finished = Cell::new(false);
    let mut items = prepared.iter();
    let items = std::iter::from_fn(|| {
        let item = items.next();
        if item.is_none() {
            finished.set(true);
        }
        item
    });
    let result = store.append_prepared_staging_iter_checked("cancel-commit:1", items, || {
        if finished.get() {
            return Err(StoreError::Conflict("commit cancelled".into()));
        }
        Ok(())
    });
    assert!(
        finished.get(),
        "must reach the end of the actual batch before cancellation"
    );
    assert!(
        matches!(result, Err(StoreError::Conflict(ref message)) if message == "commit cancelled")
    );
    assert_eq!(store.staging_node_count("cancel-commit:1").unwrap(), 0);
    assert_eq!(
        store.staging_node_count("previous:1").unwrap(),
        graph.nodes.len() as u64
    );
    let counts:(i64,i64)=store.connection.query_row("SELECT (SELECT COUNT(*) FROM scan_staging_search WHERE job_id='cancel-commit:1'),(SELECT COUNT(*) FROM scan_staging_search WHERE job_id='previous:1')",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(counts, (0, graph.nodes.len() as i64));
}
