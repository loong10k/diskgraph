use crate::StoreError;
use crate::process_job_test_fixtures::{epoch, fixture};
use diskgraph_core::{QueryBudget, QueryReadBudget, UnixFileObservation, UnixObservationGap};
fn reads(bytes: usize) -> QueryReadBudget {
    QueryReadBudget::new(
        QueryBudget {
            max_nodes: 1,
            max_response_bytes: bytes,
            ..QueryBudget::default()
        },
        std::time::Instant::now() + std::time::Duration::from_secs(1),
    )
    .unwrap()
}
#[test]
fn captured_unix_epoch_is_published_atomically_and_legacy_is_missing() {
    let (_, graph, _, _) = fixture();
    let stored = graph
        .unix_observation_bounded("process-snapshot", 2, &mut reads(2048))
        .unwrap()
        .unwrap();
    assert_eq!(stored.observation.unwrap().epoch(), &epoch());
    assert_eq!(stored.gap, None);
    let root = graph
        .unix_observation_bounded("process-snapshot", 1, &mut reads(2048))
        .unwrap()
        .unwrap();
    assert_eq!(root.observation, None);
    assert_eq!(root.gap, Some(UnixObservationGap::NotCaptured));
    assert!(matches!(
        graph.unix_observation_bounded("process-snapshot", 2, &mut reads(1)),
        Err(StoreError::BudgetExceeded)
    ));
}
#[test]
fn staging_requires_the_real_existing_ordinary_file_and_last_check() {
    let (_, mut graph, _, _) = fixture();
    let observation =
        UnixFileObservation::new(epoch(), 0o100600, 100, (1, 2), (3, 4), (10, 11)).unwrap();
    assert!(matches!(
        graph.append_staging_unix_observations_checked(
            "missing",
            std::iter::once((2, Some(&observation), None)),
            || Ok(())
        ),
        Err(StoreError::InvalidGraph(_))
    ));
    assert_eq!(
        crate::process_job_test_fixtures::count(&graph, "scan_staging_unix_observations"),
        0
    );
    assert!(matches!(
        graph.append_staging_unix_observations_checked("missing", std::iter::empty(), || Err(
            StoreError::Conflict("cancelled".into())
        )),
        Err(StoreError::Conflict(_))
    ));
}

#[test]
fn actual_unix_staging_insert_is_rolled_back_when_its_final_check_refuses() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (_, mut store, _, _) = fixture();
    let mut graph = crate::tests::graph("next-snapshot", 200);
    graph.nodes[0].directories = 0;
    graph.nodes[1].kind = diskgraph_core::NodeKind::File;
    graph.nodes[1].directories = 0;
    let locator = diskgraph_core::QualifiedLocator::from_parts(
        diskgraph_core::LocatorKind::NativePath,
        diskgraph_core::LocatorEncoding::UnixBytes,
        b"/tmp/diskgraph-test/cache".to_vec(),
        "/tmp/diskgraph-test/cache".into(),
    )
    .unwrap();
    store
        .append_staging_located_iter(
            "next-stage",
            std::iter::once((&graph.nodes[1], &locator, Some(1))),
        )
        .unwrap();
    let observation =
        UnixFileObservation::new(epoch(), 0o100600, 100, (1, 2), (3, 4), (10, 11)).unwrap();
    let inserted = Arc::new(AtomicBool::new(false));
    let mark = inserted.clone();
    store
        .connection
        .update_hook(Some(
            move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                if action == rusqlite::hooks::Action::SQLITE_INSERT
                    && table == "scan_staging_unix_observations"
                {
                    mark.store(true, Ordering::SeqCst);
                }
            },
        ))
        .unwrap();
    let result = store.append_staging_unix_observations_checked(
        "next-stage",
        std::iter::once((2, Some(&observation), None)),
        || {
            if inserted.load(Ordering::SeqCst) {
                Err(StoreError::Conflict("terminal cancellation".into()))
            } else {
                Ok(())
            }
        },
    );
    assert!(inserted.load(Ordering::SeqCst));
    assert!(matches!(result, Err(StoreError::Conflict(_))));
    assert_eq!(
        crate::process_job_test_fixtures::count(&store, "scan_staging_unix_observations"),
        0
    );
    assert_eq!(store.staging_node_count("next-stage").unwrap(), 1);
    store
        .connection
        .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)
        .unwrap();
    store
        .append_staging_unix_observations_checked(
            "next-stage",
            std::iter::once((2, Some(&observation), None)),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        crate::process_job_test_fixtures::count(&store, "scan_staging_unix_observations"),
        1
    );
    store.clear_staging("next-stage").unwrap();
    assert_eq!(
        crate::process_job_test_fixtures::count(&store, "scan_staging_unix_observations"),
        0
    );
}
