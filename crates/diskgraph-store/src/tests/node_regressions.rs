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
