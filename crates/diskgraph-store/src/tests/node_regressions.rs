use crate::SqliteSnapshotStore;
use diskgraph_core::DiskNode;
use rusqlite::Connection;

use super::fixtures::graph;
#[test]
fn snapshots_are_immutable_and_queries_are_ordered() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let first = graph("one", 100);
    let second = graph("two", 150);
    store.save(&first).unwrap();
    store.save(&second).unwrap();
    assert!(store.save(&first).is_err());
    assert_eq!(store.load("one").unwrap(), first);
    assert_eq!(
        store.latest_snapshot_id(&first.snapshot.root).unwrap(),
        Some("two".into())
    );
    assert_eq!(store.top("two", 1, 1).unwrap()[0].id, 2);
    assert!(store.children("two", 1, 1, 1).unwrap().is_empty());
    assert_eq!(store.evidence("two", 2).unwrap().len(), 1);
    assert_eq!(
        store
            .node_by_locator("two", &second.nodes[1].locator)
            .unwrap()
            .unwrap()
            .subtree_bytes,
        150
    );
}

#[test]
fn rejects_invalid_graph_without_persisting_a_partial_snapshot() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut invalid = graph("invalid", 100);
    invalid.evidence[0].node_id = 999;
    assert!(store.save(&invalid).is_err());
    assert!(store.snapshot("invalid").is_err());
}

#[test]
fn a_measured_node_stores_no_payload_and_still_reads_back_identically() {
    let dir = std::env::temp_dir().join(format!("diskgraph-payload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diskgraph.sqlite");
    let original = graph("slim", 4_096);
    {
        let mut store = SqliteSnapshotStore::open(&path).unwrap();
        store
            .publish_revision("job", &original, "rev-0", 1_700_000_000)
            .unwrap();
    }
    let connection = Connection::open(&path).unwrap();
    let stored: String = connection
        .query_row(
            "SELECT node_json FROM nodes WHERE snapshot_id = 'slim' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        stored.is_empty(),
        "a measured row must not repeat itself: {stored}"
    );
    // Every reader goes through the columns now, and they must return the
    // same node the writer was handed: the payload was a copy, and a
    // copy that drifts is worse than no copy.
    let store = SqliteSnapshotStore::open(&path).unwrap();
    assert_eq!(store.load("slim").unwrap(), original);
    assert_eq!(store.node("slim", 2).unwrap().unwrap(), original.nodes[1]);
    assert_eq!(store.root_node("slim").unwrap().unwrap(), original.nodes[0]);
    assert_eq!(
        store.children("slim", 1, 0, 10).unwrap(),
        vec![original.nodes[1].clone()]
    );
    assert_eq!(
        store
            .node_by_locator("slim", &original.nodes[1].locator)
            .unwrap()
            .unwrap(),
        original.nodes[1]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_node_of_unknown_size_keeps_its_payload_because_it_is_the_record() {
    let dir = std::env::temp_dir().join(format!("diskgraph-unknown-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diskgraph.sqlite");
    // A node whose size could not be measured has its fields nowhere
    // else: the columns are only written for fully measured nodes, so
    // dropping its payload would erase it.
    let mut unknown = graph("partial", 100);
    unknown.nodes[1].size_known = false;
    {
        let mut store = SqliteSnapshotStore::open(&path).unwrap();
        store
            .publish_revision("job", &unknown, "rev-0", 1_700_000_000)
            .unwrap();
    }
    let store = SqliteSnapshotStore::open(&path).unwrap();
    assert_eq!(store.node("partial", 2).unwrap().unwrap(), unknown.nodes[1]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn v4_structured_rows_read_identically_to_their_json_payload() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshots.sqlite");
    // One node of each shape: a measured one that lives in the columns,
    // and one whose payload is the only record of it.
    let mut mixed = graph("v4", 4096);
    mixed.nodes[1].size_known = false;
    let mut store = SqliteSnapshotStore::open(&path).unwrap();
    store.save(&mixed).unwrap();
    drop(store);

    // The `kind` column is the marker for which shape a row is in: set
    // means the columns carry the node, empty means the payload does.
    let connection = rusqlite::Connection::open(&path).unwrap();
    let rows: Vec<(String, Option<String>)> = connection
        .prepare("SELECT node_json, kind FROM nodes WHERE snapshot_id = ?1")
        .unwrap()
        .query_map([&mixed.snapshot.id], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    let measured = rows
        .iter()
        .find(|(_, kind)| kind.is_some())
        .expect("a measured row carries the columns");
    assert!(measured.0.is_empty(), "and no copy of itself");
    let unmeasured = rows
        .iter()
        .find(|(_, kind)| kind.is_none())
        .expect("an unmeasured row keeps its payload");
    let parsed: DiskNode = serde_json::from_str(&unmeasured.0).unwrap();
    assert!(!parsed.size_known);

    // Both shapes read back as the nodes that were written.
    let store = SqliteSnapshotStore::open(&path).unwrap();
    let loaded = store.load(&mixed.snapshot.id).unwrap();
    assert_eq!(loaded.nodes.len(), mixed.nodes.len());
    for (loaded, written) in loaded.nodes.iter().zip(mixed.nodes.iter()) {
        assert_eq!(loaded, written, "the fast path must lose nothing");
    }
}

#[test]
fn narrow_reads_refuse_wrong_column_types_instead_of_defaulting_them() {
    // SQLite 的非严格列允许存入其他类型；窄读必须传播解码错误，不能将它变成零。
    for column in [
        "direct_bytes",
        "files",
        "directories",
        "modified_unix_seconds",
        "file_id",
        "read_error",
        "name",
        "category_hint",
        "reclaim_hint",
    ] {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let original = graph("typed", 4096);
        store.save(&original).unwrap();
        let invalid = if matches!(column, "name" | "category_hint" | "reclaim_hint") {
            "X'FF'"
        } else {
            "'not-an-integer'"
        };
        store
            .connection
            .execute(
                &format!("UPDATE nodes SET {column}={invalid} WHERE snapshot_id='typed' AND id=2"),
                [],
            )
            .unwrap();
        assert!(
            store.node("typed", 2).is_err(),
            "node defaulted invalid {column}"
        );
        assert!(
            store.children("typed", 1, 0, 10).is_err(),
            "children defaulted invalid {column}"
        );
        // read_error 不为 0 的行不符合 known 页谓词；验证实际会返回的行。
        if column != "read_error" {
            assert!(
                store.children_page("typed", 1, None, 0, 10).is_err(),
                "page defaulted invalid {column}"
            );
        }
        assert!(
            store
                .with_ordered_nodes("typed", "/fixture", |rows| {
                    rows.collect::<crate::Result<Vec<_>>>()
                })
                .is_err(),
            "history defaulted invalid {column}"
        );
    }
}

#[test]
fn narrow_legacy_reads_refuse_payload_id_that_disagrees_with_the_row() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut original = graph("legacy-id", 4096);
    original.nodes[1].size_known = false;
    store.save(&original).unwrap();
    let mut corrupted = original.nodes[1].clone();
    corrupted.id = 77;
    store
        .connection
        .execute(
            "UPDATE nodes SET node_json=?1 WHERE snapshot_id='legacy-id' AND id=2",
            [serde_json::to_string(&corrupted).unwrap()],
        )
        .unwrap();
    assert!(
        store.node("legacy-id", 2).is_err(),
        "a row cannot authorize another payload ID"
    );
    assert!(store.children("legacy-id", 1, 0, 10).is_err());
}

#[test]
fn actual_point_lookup_vm_work_scales_with_selection_not_snapshot_size() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let mut results = Vec::new();
    for count in [20_000_u64, 200_000] {
        let mut fixture = graph("point-work", 4096);
        let template = fixture.nodes[1].clone();
        for id in 3..=count {
            let mut node = template.clone();
            node.id = id;
            node.name = format!("metadata-{id}");
            node.locator =
                diskgraph_core::ResourceLocator::NativePath(format!("/fixture/metadata-{id}"));
            node.file_identity = None;
            fixture.nodes.push(node);
        }
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        store.save(&fixture).unwrap();
        drop(fixture);
        let steps = Arc::new(AtomicU64::new(0));
        let observed = steps.clone();
        store
            .connection
            .progress_handler(
                1,
                Some(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        assert_eq!(store.node("point-work", 2).unwrap().unwrap().id, 2);
        let one = steps.swap(0, Ordering::Relaxed);
        for id in 2..18 {
            assert_eq!(store.node("point-work", id).unwrap().unwrap().id, id);
        }
        let sixteen = steps.load(Ordering::Relaxed);
        store
            .connection
            .progress_handler(0, None::<fn() -> bool>)
            .unwrap();
        println!("POINT_NODE_VM rows={count} selected=1 steps={one} selected=16 steps={sixteen}");
        assert!(one > 0 && sixteen >= one * 12 && sixteen <= one * 20);
        results.push((one, sixteen));
    }
    assert!(
        results[1].0 <= results[0].0 + 512,
        "one lookup walked the larger snapshot: {results:?}"
    );
    assert!(
        results[1].1 <= results[0].1 + 512,
        "selection walked the larger snapshot: {results:?}"
    );
}
