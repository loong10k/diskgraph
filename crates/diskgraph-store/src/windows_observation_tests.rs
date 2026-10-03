//! D31 Windows 原生观测持久化与隔离回归。

use crate::{SqliteSnapshotStore, tests::graph};

#[test]
fn windows_observation_schema_exists_without_inventing_legacy_metadata() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&graph("legacy", 100)).unwrap();
    for table in ["nodes", "scan_staging"] {
        let columns:i64 = store.connection.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name IN ('native_observation_format','native_observation_raw','native_observation_gap')"),[],|row|row.get(0)).unwrap();
        assert_eq!(
            columns, 3,
            "{table} cannot retain full Windows native observation"
        );
    }
    let invented:i64=store.connection.query_row("SELECT COUNT(*) FROM nodes WHERE native_observation_format IS NOT NULL OR native_observation_raw IS NOT NULL OR native_observation_gap IS NOT NULL",[],|row|row.get(0)).unwrap();
    assert_eq!(
        invented, 0,
        "legacy v1 fields cannot establish native Windows observation"
    );
}

use crate::windows_observation_fixtures::{budget, fixture, observation, stage, store};
use crate::{StoreError, staging_node_encoded_cost, staging_observed_node_encoded_cost};
use diskgraph_core::{QualifiedLocator, WindowsFileObservation, WindowsObservationGap};

#[test]
fn windows_observation_roundtrip_reopen_preserves_full_bits_and_old_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let mut store = SqliteSnapshotStore::open(&path).unwrap();
    let (graph, locators) = fixture("observed");
    stage(&mut store, "job", &graph, &locators);
    store
        .publish_revision_owned("job", &graph, "revision", 1, Some(("server", "scope")))
        .unwrap();
    assert_eq!(store.staging_node_count("job").unwrap(), 0);
    drop(store);
    let mut store = SqliteSnapshotStore::open(&path).unwrap();
    let value = store
        .windows_observation_bounded("observed", 2, &mut budget(4096, 1))
        .unwrap()
        .unwrap();
    assert_eq!(value.observation, Some(observation()));
    assert_eq!(value.gap, None);
    assert!(
        store
            .windows_observation_bounded("observed", 99, &mut budget(4096, 1))
            .unwrap()
            .is_none()
    );
    store.save(&crate::tests::graph("legacy", 101)).unwrap();
    let value = store
        .windows_observation_bounded("legacy", 2, &mut budget(4096, 1))
        .unwrap()
        .unwrap();
    assert_eq!(value.observation, None);
    assert_eq!(value.gap, Some(WindowsObservationGap::NotCaptured));
    let generations:(i64,i64,i64,i64)=store.connection.query_row("SELECT r.writer_generation,r.locator_writer_generation,r.native_observation_writer_generation,s.count_schema FROM graph_revisions r JOIN snapshots s ON s.id=r.snapshot_id WHERE r.revision_id='revision'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(generations, (10, 11, 12, 9));
}

#[test]
fn windows_observation_staging_cost_matches_actual_fields_and_gap_is_portable() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let (graph, locators) = fixture("observed");
    let node = &graph.nodes[0];
    let locator = &locators[0];
    let obs = observation();
    let old = staging_node_encoded_cost(node, Some(locator), Some(42)).unwrap();
    let new = staging_observed_node_encoded_cost(node, Some(locator), Some(42), Some(&obs), None)
        .unwrap();
    assert_eq!(
        new - old,
        (WindowsFileObservation::ENCODED_LEN + WindowsFileObservation::FORMAT_LABEL.len()) as u64
    );
    store
        .append_staging_observed_iter(
            "job",
            std::iter::once((node, locator, Some(42), Some(&obs), None)),
        )
        .unwrap();
    let sql_cost:i64=store.connection.query_row("SELECT length(CAST(s.node_json AS BLOB))+length(CAST(q.name_fold AS BLOB))+length(CAST(q.path_fold AS BLOB))+length(CAST(s.native_locator_kind AS BLOB))+length(CAST(s.native_locator_encoding AS BLOB))+length(s.native_locator_raw)+8+length(CAST(s.native_observation_format AS BLOB))+length(s.native_observation_raw) FROM scan_staging s JOIN scan_staging_search q USING(job_id,node_seq)",[],|r|r.get(0)).unwrap();
    assert_eq!(new, sql_cost as u64);
    let native =
        QualifiedLocator::from_native_path(std::path::Path::new("/tmp/diskgraph-test")).unwrap();
    assert_eq!(
        staging_observed_node_encoded_cost(
            node,
            Some(&native),
            None,
            None,
            Some(WindowsObservationGap::Unsupported)
        )
        .unwrap()
            - staging_node_encoded_cost(node, Some(&native), None).unwrap(),
        "unsupported".len() as u64
    );
    store
        .append_staging_observed_iter(
            "gap",
            std::iter::once((
                node,
                &native,
                None,
                None,
                Some(WindowsObservationGap::Unsupported),
            )),
        )
        .unwrap();
}

#[test]
fn windows_observation_invalid_batch_rolls_back_preceding_rows_and_search() {
    let (graph, locators) = fixture("observed");
    let obs = observation();
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let rows = [
        (&graph.nodes[0], &locators[0], None, Some(&obs), None),
        (
            &graph.nodes[1],
            &locators[1],
            None,
            Some(&obs),
            Some(WindowsObservationGap::Changed),
        ),
    ];
    assert!(
        store
            .append_staging_observed_iter("bad", rows.into_iter())
            .is_err()
    );
    assert_eq!(store.staging_node_count("bad").unwrap(), 0);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM scan_staging_search WHERE job_id='bad'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let unix = diskgraph_core::QualifiedLocator::from_parts(
        diskgraph_core::LocatorKind::NativePath,
        diskgraph_core::LocatorEncoding::UnixBytes,
        b"/tmp/diskgraph-test".to_vec(),
        "/tmp/diskgraph-test".into(),
    )
    .unwrap();
    assert!(
        staging_observed_node_encoded_cost(&graph.nodes[0], Some(&unix), None, Some(&obs), None)
            .is_err()
    );
    assert!(
        staging_observed_node_encoded_cost(&graph.nodes[0], None, None, Some(&obs), None).is_err()
    );
}

#[test]
fn windows_observation_query_rejects_half_columns_types_unknown_formats_and_bad_codec() {
    for assignment in [
        "native_observation_format=NULL",
        "native_observation_raw=NULL",
        "native_observation_gap='changed'",
        "native_observation_format=x'00'",
        "native_observation_raw='text'",
        "native_observation_format='windows_file_observation_v2'",
        "native_observation_raw=zeroblob(79)",
        "native_observation_raw=zeroblob(81)",
        "native_observation_raw=zeroblob(80)",
        "native_locator_kind='document_uri'",
        "native_locator_encoding='unix_bytes'",
        "native_locator_encoding=NULL",
    ] {
        let store = store();
        store
            .connection
            .execute(
                &format!("UPDATE nodes SET {assignment} WHERE snapshot_id='observed' AND id=2"),
                [],
            )
            .unwrap();
        assert!(
            store
                .windows_observation_bounded("observed", 2, &mut budget(4096, 1))
                .is_err(),
            "accepted {assignment}"
        );
    }
    for gap in ["'unknown_gap'", "x'00'", "37"] {
        let store = store();
        store.connection.execute(&format!("UPDATE nodes SET native_observation_format=NULL,native_observation_raw=NULL,native_observation_gap={gap} WHERE id=2"),[]).unwrap();
        assert!(
            store
                .windows_observation_bounded("observed", 2, &mut budget(4096, 1))
                .is_err()
        );
    }
}

#[test]
fn windows_observation_budget_admits_borrowed_blob_before_decode_and_accumulates() {
    let store = store();
    let bytes = WindowsFileObservation::FORMAT_LABEL.len()
        + 80
        + "native_path".len()
        + "windows_utf16_le".len();
    assert!(
        store
            .windows_observation_bounded("observed", 2, &mut budget(bytes, 1))
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        store.windows_observation_bounded("observed", 2, &mut budget(bytes - 1, 1)),
        Err(StoreError::BudgetExceeded)
    ));
    let mut shared = budget(bytes * 2 - 1, 2);
    store
        .windows_observation_bounded("observed", 2, &mut shared)
        .unwrap();
    assert!(matches!(
        store.windows_observation_bounded("observed", 2, &mut shared),
        Err(StoreError::BudgetExceeded)
    ));
    let mut nodes = budget(4096, 1);
    store
        .windows_observation_bounded("observed", 2, &mut nodes)
        .unwrap();
    assert!(matches!(
        store.windows_observation_bounded("observed", 2, &mut nodes),
        Err(StoreError::BudgetExceeded)
    ));
    store.connection.execute("UPDATE nodes SET native_observation_raw=zeroblob(1000000),native_observation_format='invalid' WHERE id=2",[]).unwrap();
    assert!(matches!(
        store.windows_observation_bounded("observed", 2, &mut budget(4096, 1)),
        Err(StoreError::BudgetExceeded)
    ));
    let mut expired = diskgraph_core::QueryReadBudget::new(
        diskgraph_core::QueryBudget::default(),
        std::time::Instant::now(),
    )
    .unwrap();
    assert!(matches!(
        store.windows_observation_bounded("observed", 2, &mut expired),
        Err(StoreError::BudgetExceeded)
    ));
}
