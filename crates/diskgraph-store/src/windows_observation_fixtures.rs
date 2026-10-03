//! D31 独立测试夹具。
use crate::{SqliteSnapshotStore, tests::graph};
use diskgraph_core::{
    DiskGraph, LocatorEncoding, LocatorKind, QualifiedLocator, QueryBudget, QueryReadBudget,
    ResourceLocator, WindowsFileObservation, WindowsTreeAlignment,
};
use std::time::{Duration, Instant};

pub(super) fn observation() -> WindowsFileObservation {
    WindowsFileObservation {
        volume: u64::MAX,
        file_id: [0xfe; 16],
        length: i64::MAX as u64,
        creation_time: i64::MIN + 1,
        last_write_time: 12_345_678,
        change_time: 12_345_679,
        attributes: 0x20,
        directory: false,
        delete_pending: false,
        capture_started_unix_ms: 100,
        capture_finished_unix_ms: 101,
        tree_alignment: WindowsTreeAlignment::Unverified,
    }
}
pub(super) fn budget(bytes: usize, nodes: usize) -> QueryReadBudget {
    QueryReadBudget::new(
        QueryBudget {
            max_response_bytes: bytes,
            max_nodes: nodes,
            ..QueryBudget::default()
        },
        Instant::now() + Duration::from_secs(30),
    )
    .unwrap()
}
pub(super) fn fixture(id: &str) -> (DiskGraph, Vec<QualifiedLocator>) {
    let graph = graph(id, 100);
    let locators = graph
        .nodes
        .iter()
        .map(|node| {
            let ResourceLocator::NativePath(display) = &node.locator else {
                unreachable!()
            };
            QualifiedLocator::from_parts(
                LocatorKind::NativePath,
                LocatorEncoding::WindowsUtf16Le,
                display.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                display.clone(),
            )
            .unwrap()
        })
        .collect();
    (graph, locators)
}
pub(super) fn stage(
    store: &mut SqliteSnapshotStore,
    job: &str,
    graph: &DiskGraph,
    locators: &[QualifiedLocator],
) {
    let observation = observation();
    store
        .append_staging_observed_iter(
            job,
            graph
                .nodes
                .iter()
                .zip(locators)
                .map(|(node, locator)| (node, locator, Some(42), Some(&observation), None)),
        )
        .unwrap();
}
pub(super) fn store() -> SqliteSnapshotStore {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let (graph, locators) = fixture("observed");
    stage(&mut store, "job", &graph, &locators);
    store
        .publish_revision_owned(
            "job",
            &graph,
            "observed-revision",
            1,
            Some(("server", "scope")),
        )
        .unwrap();
    store
}
pub(super) fn downgrade_to_v11(db: &rusqlite::Connection) {
    db.execute_batch(
        "DROP TRIGGER revisions_require_native_observation_writer;
    ALTER TABLE graph_revisions DROP COLUMN native_observation_writer_generation;
    ALTER TABLE nodes DROP COLUMN native_observation_format;
    ALTER TABLE nodes DROP COLUMN native_observation_raw;
    ALTER TABLE nodes DROP COLUMN native_observation_gap;
    ALTER TABLE scan_staging DROP COLUMN native_observation_format;
    ALTER TABLE scan_staging DROP COLUMN native_observation_raw;
    ALTER TABLE scan_staging DROP COLUMN native_observation_gap;
    PRAGMA user_version=11;",
    )
    .unwrap();
}
