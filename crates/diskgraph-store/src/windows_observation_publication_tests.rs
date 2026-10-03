//! D31 迁移、写入代次、发布原子性与窄读工作量。
use crate::{
    SqliteSnapshotStore,
    windows_observation_fixtures::{budget, downgrade_to_v11, fixture, observation, stage, store},
};
use diskgraph_core::WindowsObservationGap;

#[test]
fn windows_observation_v11_backup_is_consistent_and_old_open_writer_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let mut original = SqliteSnapshotStore::open(&path).unwrap();
    original.save(&crate::tests::graph("legacy", 100)).unwrap();
    downgrade_to_v11(&original.connection);
    original.connection.execute_batch("CREATE TABLE backup_probe(value TEXT);INSERT INTO backup_probe VALUES('committed-in-wal');").unwrap();
    let old = rusqlite::Connection::open(&path).unwrap();
    let (mut current, backup) =
        SqliteSnapshotStore::open_with_backup(&path, &dir.path().join("backups")).unwrap();
    let backup = rusqlite::Connection::open(backup.unwrap()).unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert_eq!(
        backup
            .query_row("SELECT value FROM backup_probe", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "committed-in-wal"
    );
    assert_eq!(backup.query_row("SELECT COUNT(*) FROM pragma_table_info('nodes') WHERE name='native_observation_raw'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    let legacy = current
        .windows_observation_bounded("legacy", 2, &mut budget(4096, 1))
        .unwrap()
        .unwrap();
    assert_eq!(legacy.observation, None);
    assert_eq!(legacy.gap, Some(WindowsObservationGap::NotCaptured));
    let error=old.execute("INSERT INTO graph_revisions(revision_id,snapshot_id,published_at_unix_ms,writer_generation,locator_writer_generation) VALUES('obsolete','legacy',1,10,11)",[]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("obsolete native observation writer")
    );
    current
        .publish_revision(
            "current",
            &crate::tests::graph("current", 101),
            "current",
            2,
        )
        .unwrap();
    assert_eq!(current.connection.query_row("SELECT native_observation_writer_generation FROM graph_revisions WHERE revision_id='current'",[],|r|r.get::<_,i64>(0)).unwrap(),12);
}

#[test]
fn windows_observation_publication_failure_keeps_staging_and_rolls_back_latest() {
    let mut store = store();
    let (graph, locators) = fixture("next");
    stage(&mut store, "next-job", &graph, &locators);
    store.connection.execute_batch("CREATE TRIGGER fail_observation_publish BEFORE UPDATE ON latest_revision BEGIN SELECT RAISE(ABORT,'injected latest failure'); END;").unwrap();
    assert!(
        store
            .publish_revision_owned(
                "next-job",
                &graph,
                "next-revision",
                2,
                Some(("server", "scope"))
            )
            .is_err()
    );
    assert_eq!(store.staging_node_count("next-job").unwrap(), 2);
    assert_eq!(store.connection.query_row("SELECT COUNT(*) FROM scan_staging WHERE job_id='next-job' AND length(native_observation_raw)=80",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert!(store.snapshot("next").is_err());
    assert_eq!(
        store
            .latest_revision_for_root(&graph.snapshot.root)
            .unwrap()
            .unwrap(),
        "observed-revision"
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_observation_publish;")
        .unwrap();
    store
        .publish_revision_owned(
            "next-job",
            &graph,
            "next-revision",
            2,
            Some(("server", "scope")),
        )
        .unwrap();
    assert_eq!(
        store
            .windows_observation_bounded("next", 2, &mut budget(4096, 1))
            .unwrap()
            .unwrap()
            .observation,
        Some(observation())
    );
}

#[test]
fn windows_observation_point_query_work_is_independent_of_unrelated_nodes() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for unrelated in [20_000, 200_000] {
        let store = store();
        store.connection.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<?1)
        INSERT INTO nodes(snapshot_id,id,parent_id,locator_key,name,subtree_bytes,node_json,native_observation_format,native_observation_raw,native_observation_gap)
        SELECT 'observed',x+2,1,'null','unrelated',0,'null','invalid',zeroblob(100),'invalid' FROM n",[unrelated]).unwrap();
        let steps = Arc::new(AtomicUsize::new(0));
        let observed = steps.clone();
        store
            .connection
            .progress_handler(
                1,
                Some(move || observed.fetch_add(1, Ordering::Relaxed) > 500),
            )
            .unwrap();
        let result = store
            .windows_observation_bounded("observed", 2, &mut budget(4096, 1))
            .unwrap()
            .unwrap();
        store
            .connection
            .progress_handler(0, None::<fn() -> bool>)
            .unwrap();
        assert_eq!(result.observation, Some(observation()));
        let work = steps.load(Ordering::Relaxed);
        eprintln!(
            "D31 Windows observation: unrelated={unrelated}, selected=1, sqlite_vm_steps={work}"
        );
        assert!(work < 500);
    }
}

#[test]
fn windows_observation_gaps_survive_publish_and_fenced_cleanup_removes_metadata() {
    for gap in [
        WindowsObservationGap::NotCaptured,
        WindowsObservationGap::Unsupported,
        WindowsObservationGap::CaptureFailed,
        WindowsObservationGap::Changed,
        WindowsObservationGap::TreeMismatch,
    ] {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let (graph, locators) = fixture("gap");
        store
            .append_staging_observed_iter(
                "job:1",
                graph
                    .nodes
                    .iter()
                    .zip(&locators)
                    .map(|(node, locator)| (node, locator, None, None, Some(gap))),
            )
            .unwrap();
        stage(&mut store, "job:2", &graph, &locators);
        store.clear_stale_job_staging("job", 2).unwrap();
        assert_eq!(store.staging_node_count("job:1").unwrap(), 0);
        assert_eq!(store.staging_node_count("job:2").unwrap(), 2);
        store.clear_staging("job:2").unwrap();
        store
            .append_staging_observed_iter(
                "job:3",
                graph
                    .nodes
                    .iter()
                    .zip(&locators)
                    .map(|(node, locator)| (node, locator, None, None, Some(gap))),
            )
            .unwrap();
        store
            .publish_revision_owned("job:3", &graph, "gap", 1, Some(("server", "scope")))
            .unwrap();
        let result = store
            .windows_observation_bounded("gap", 2, &mut budget(4096, 1))
            .unwrap()
            .unwrap();
        assert_eq!(result.observation, None);
        assert_eq!(result.gap, Some(gap));
    }
}
