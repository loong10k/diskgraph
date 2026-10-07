//! D42 Unix 观测真实暂存点查成本与目标准入；只使用隔离内存图库和公开暂存 API。
//! VM 计数来自 progress_handler(1)，不代表 RSS、SQLite C 分配或实际磁盘 I/O。
use crate::{SqliteSnapshotStore, StoreError};
use diskgraph_core::{
    DiskNode, LocatorEncoding, LocatorKind, NodeKind, QualifiedLocator, ResourceLocator,
    UnixFileObservation,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn node_and_locator() -> (DiskNode, QualifiedLocator) {
    let mut node = crate::tests::graph("staging-point-fixture", 100)
        .nodes
        .remove(1);
    node.kind = NodeKind::File;
    node.directories = 0;
    let locator = QualifiedLocator::from_parts(
        LocatorKind::NativePath,
        LocatorEncoding::UnixBytes,
        b"/tmp/diskgraph-test/cache".to_vec(),
        "/tmp/diskgraph-test/cache".into(),
    )
    .unwrap();
    (node, locator)
}

fn observation() -> UnixFileObservation {
    UnixFileObservation::new(
        crate::process_job_test_fixtures::epoch(),
        0o100600,
        100,
        (1, 2),
        (3, 4),
        (10, 11),
    )
    .unwrap()
}

fn staged_store(rows: u64) -> SqliteSnapshotStore {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let (template, locator) = node_and_locator();
    // 使用真实编码/暂存事务，有限批次不会在 Rust 中持有整个 200k 节点集合。
    for first in (1..=rows).step_by(512) {
        let nodes: Vec<_> = (first..=(first + 511).min(rows))
            .map(|id| {
                let mut node = template.clone();
                node.id = id;
                node
            })
            .collect();
        store
            .append_staging_located_iter(
                "point-stage",
                nodes.iter().map(|node| (node, &locator, Some(1))),
            )
            .unwrap();
    }
    assert_eq!(store.staging_node_count("point-stage").unwrap(), rows);
    store
}

fn assert_point_cost(store: &mut SqliteSnapshotStore, rows: u64) {
    let vm = Arc::new(AtomicU64::new(0));
    let measured = Arc::clone(&vm);
    store
        .connection
        .progress_handler(
            1,
            Some(move || {
                measured.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let observed = observation();
    let outcome = store.append_staging_unix_observations_checked(
        "point-stage",
        [1, rows / 2, rows]
            .into_iter()
            .map(|id| (id, Some(&observed), None)),
        || Ok(()),
    );
    // 仅测量真实三个观测点查及旁表 INSERT/commit；不把全表正控计数算进点查。
    let steps = vm.load(Ordering::Relaxed);
    store
        .connection
        .progress_handler(0, None::<fn() -> bool>)
        .unwrap();
    outcome.unwrap();
    assert_eq!(
        crate::process_job_test_fixtures::count(store, "scan_staging_unix_observations"),
        3
    );
    eprintln!("process Unix staging rows={rows}; points=3; exact_vm_steps={steps}");
    assert!(
        steps < 1500,
        "three point admissions scanned {rows} staging rows: {steps} VM steps"
    );
}

#[test]
fn unix_staging_three_points_do_not_walk_twenty_thousand_other_nodes() {
    assert_point_cost(&mut staged_store(20_000), 20_000);
}

#[test]
fn unix_staging_three_points_do_not_walk_two_hundred_thousand_other_nodes() {
    assert_point_cost(&mut staged_store(200_000), 200_000);
}

fn first_staged_json(store: &SqliteSnapshotStore) -> String {
    store
        .connection
        .query_row(
            "SELECT node_json FROM scan_staging WHERE job_id='point-stage' AND node_seq=1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn unix_staging_v13_upgrade_indexes_existing_rows_without_rewriting_them() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let original = staged_store(20_000);
    let raw = first_staged_json(&original);
    original
        .connection
        .backup(rusqlite::MAIN_DB, &path, None)
        .unwrap();
    {
        let old = rusqlite::Connection::open(&path).unwrap();
        // 构造真正 v13 结构；旧暂存数据仍由原公开 writer 编码，不伪造节点 JSON。
        old.execute_batch(
            "DROP VIEW IF EXISTS revision_authorized_ownership; DROP TABLE IF EXISTS revision_access_denials; DROP INDEX IF EXISTS scan_staging_by_job_node_id;
             DROP TRIGGER process_receipt_no_update;
             DROP TRIGGER process_receipt_no_delete;
             DROP TRIGGER process_receipt_no_git_job;
             DROP TRIGGER git_receipt_no_process_job;
             DROP TABLE process_job_publication_receipts;
             DROP TABLE node_unix_observations;
             DROP TABLE scan_staging_unix_observations;
             PRAGMA user_version=13;",
        )
        .unwrap();
    }
    let (mut upgraded, backup) =
        SqliteSnapshotStore::open_with_backup(&path, &directory.path().join("backups")).unwrap();
    assert_eq!(upgraded.staging_node_count("point-stage").unwrap(), 20_000);
    assert_eq!(first_staged_json(&upgraded), raw);
    let backup = rusqlite::Connection::open(backup.unwrap()).unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        13
    );
    assert_eq!(
        backup
            .query_row("SELECT COUNT(*) FROM scan_staging", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        20_000
    );
    assert_point_cost(&mut upgraded, 20_000);
}

#[test]
fn unix_staging_current_graph_reopen_indexes_existing_rows_without_rewriting_them() {
    let original = staged_store(20_000);
    let raw = first_staged_json(&original);
    original
        .connection
        .execute_batch("DROP INDEX IF EXISTS scan_staging_by_job_node_id;")
        .unwrap();
    assert_eq!(
        original
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        crate::SUPPORTED_SCHEMA_VERSION
    );
    let mut reopened = SqliteSnapshotStore::initialize(original.connection).unwrap();
    assert_eq!(reopened.staging_node_count("point-stage").unwrap(), 20_000);
    assert_eq!(first_staged_json(&reopened), raw);
    assert_point_cost(&mut reopened, 20_000);
}

#[test]
fn unix_staging_duplicate_ids_and_missing_ids_are_refused_without_observation_rows() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let (node, locator) = node_and_locator();
    // 可信旧暂存入口允许重复值；观测准入必须拒绝歧义，不能用 LIMIT 1 隐去重复。
    store
        .append_staging_located_iter(
            "ambiguous-stage",
            [(&node, &locator, Some(1)), (&node, &locator, Some(1))].into_iter(),
        )
        .unwrap();
    let observed = observation();
    for (id, expected_overflow) in [(node.id, false), (99, false), (u64::MAX, true)] {
        let result = store.append_staging_unix_observations_checked(
            "ambiguous-stage",
            std::iter::once((id, Some(&observed), None)),
            || Ok(()),
        );
        assert!(
            if expected_overflow {
                matches!(result, Err(StoreError::IntegerOverflow))
            } else {
                matches!(result, Err(StoreError::InvalidGraph(_)))
            },
            "invalid target {id}: {result:?}"
        );
    }
    assert_eq!(store.staging_node_count("ambiguous-stage").unwrap(), 2);
    assert_eq!(
        crate::process_job_test_fixtures::count(&store, "scan_staging_unix_observations"),
        0
    );
}

#[test]
fn unix_staging_target_requires_ordinary_readable_and_lossless_unix_metadata() {
    let observed = observation();
    for invalid in [
        "directory",
        "read_error",
        "missing_native_locator",
        "foreign_native_encoding",
    ] {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let (mut node, mut locator) = node_and_locator();
        match invalid {
            "directory" => node.kind = NodeKind::Directory,
            "read_error" => node.read_error = true,
            "foreign_native_encoding" => {
                let path = r"C:\fixture\target";
                node.locator = ResourceLocator::NativePath(path.into());
                locator = QualifiedLocator::from_parts(
                    LocatorKind::NativePath,
                    LocatorEncoding::WindowsUtf16Le,
                    path.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                    path.into(),
                )
                .unwrap();
            }
            _ => {}
        }
        if invalid == "missing_native_locator" {
            store
                .append_staging_nodes("invalid-stage", &[node.clone()])
                .unwrap();
        } else {
            store
                .append_staging_located_iter(
                    "invalid-stage",
                    std::iter::once((&node, &locator, Some(1))),
                )
                .unwrap();
        }
        let result = store.append_staging_unix_observations_checked(
            "invalid-stage",
            std::iter::once((node.id, Some(&observed), None)),
            || Ok(()),
        );
        assert!(
            matches!(result, Err(StoreError::InvalidGraph(_))),
            "{invalid}: {result:?}"
        );
        assert_eq!(
            crate::process_job_test_fixtures::count(&store, "scan_staging_unix_observations"),
            0
        );
    }
}
