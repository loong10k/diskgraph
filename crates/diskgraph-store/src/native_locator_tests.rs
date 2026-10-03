//! D30 无损定位持久化：独占内存/临时数据库回归。

use crate::{SqliteSnapshotStore, tests::graph};

#[test]
fn located_node_schema_is_present_without_guessing_legacy_bytes() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&graph("legacy", 100)).unwrap();
    for table in ["nodes", "scan_staging"] {
        let columns: i64 = store.connection.query_row(
            &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name IN ('native_locator_kind','native_locator_encoding','native_locator_raw','self_modified_unix_seconds')"), [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            columns, 4,
            "{table} lacks encoding-qualified raw locator fields"
        );
    }
    let known: i64 = store.connection.query_row("SELECT COUNT(*) FROM nodes WHERE native_locator_raw IS NOT NULL OR native_locator_kind IS NOT NULL OR native_locator_encoding IS NOT NULL OR self_modified_unix_seconds IS NOT NULL",[],|row| row.get(0)).unwrap();
    assert_eq!(
        known, 0,
        "v1 writers must not invent raw paths from display strings"
    );
}

use crate::{StoreError, staging_node_encoded_cost};
use diskgraph_core::{QualifiedLocator, QueryBudget, QueryReadBudget, ResourceLocator};
use rusqlite::params;
use std::time::{Duration, Instant};

fn budget(bytes: usize, nodes: usize) -> QueryReadBudget {
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

fn located_store() -> SqliteSnapshotStore {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = graph("located", 100);
    let locators: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| match &node.locator {
            ResourceLocator::NativePath(path) => {
                QualifiedLocator::from_native_path(std::path::Path::new(path)).unwrap()
            }
            _ => unreachable!(),
        })
        .collect();
    store
        .append_staging_located_iter(
            "job",
            graph
                .nodes
                .iter()
                .zip(&locators)
                .map(|(node, locator)| (node, locator, Some(42))),
        )
        .unwrap();
    store
        .publish_revision_owned(
            "job",
            &graph,
            "located-revision",
            1,
            Some(("server", "scope")),
        )
        .unwrap();
    store
}

#[test]
fn exact_native_locator_preserves_raw_and_self_time_and_legacy_unavailable() {
    let mut store = located_store();
    let result = store
        .native_locator_bounded("located", 2, &mut budget(4096, 1))
        .unwrap()
        .unwrap();
    assert_eq!(result.self_modified, Some(42));
    assert_eq!(
        result.locator.unwrap().to_native_path().unwrap(),
        std::path::Path::new("/tmp/diskgraph-test/cache")
    );
    assert!(
        store
            .native_locator_bounded("located", 999, &mut budget(4096, 1))
            .unwrap()
            .is_none()
    );
    store.save(&graph("legacy", 100)).unwrap();
    let legacy = store
        .native_locator_bounded("legacy", 2, &mut budget(4096, 1))
        .unwrap()
        .unwrap();
    assert!(legacy.locator.is_none());
    assert_eq!(legacy.self_modified, None);
}

#[test]
fn locator_budget_checks_raw_before_decoding_and_accumulates_nodes_bytes_deadline() {
    let store = located_store();
    let cost: i64 = store.connection.query_row("SELECT length(CAST(native_locator_kind AS BLOB))+length(CAST(native_locator_encoding AS BLOB))+length(native_locator_raw)+length(CAST(locator_key AS BLOB))+8 FROM nodes WHERE snapshot_id='located' AND id=2",[],|row|row.get(0)).unwrap();
    assert!(
        store
            .native_locator_bounded("located", 2, &mut budget(cost as usize, 1))
            .unwrap()
            .is_some()
    );
    let mut bytes = budget(cost as usize * 2 - 1, 10);
    store
        .native_locator_bounded("located", 2, &mut bytes)
        .unwrap();
    assert!(matches!(
        store.native_locator_bounded("located", 2, &mut bytes),
        Err(StoreError::BudgetExceeded)
    ));
    let mut nodes = budget(4096, 1);
    store
        .native_locator_bounded("located", 2, &mut nodes)
        .unwrap();
    assert!(matches!(
        store.native_locator_bounded("located", 2, &mut nodes),
        Err(StoreError::BudgetExceeded)
    ));
    let mut expired = QueryReadBudget::new(
        QueryBudget::default(),
        Instant::now() - Duration::from_secs(1),
    )
    .unwrap();
    assert!(matches!(
        store.native_locator_bounded("missing", 99, &mut expired),
        Err(StoreError::BudgetExceeded)
    ));
    store.connection.execute("UPDATE nodes SET native_locator_raw=zeroblob(100000),native_locator_encoding='not-an-encoding',locator_key='null' WHERE snapshot_id='located' AND id=2",[]).unwrap();
    assert!(matches!(
        store.native_locator_bounded("located", 2, &mut budget(4096, 1)),
        Err(StoreError::BudgetExceeded)
    ));
}

#[test]
fn locator_half_columns_bad_types_unknown_encoding_and_wrong_display_are_rejected() {
    for change in [
        "native_locator_raw=NULL",
        "native_locator_encoding='unknown'",
        "native_locator_kind='other'",
        "native_locator_raw='text-is-not-blob'",
        "self_modified_unix_seconds='not-an-integer'",
        "locator_key='null'",
        r#"locator_key='{"type":"native_path","value":"/forged"}'"#,
    ] {
        let store = located_store();
        store
            .connection
            .execute(
                &format!("UPDATE nodes SET {change} WHERE snapshot_id='located' AND id=2"),
                [],
            )
            .unwrap();
        let result = store.native_locator_bounded("located", 2, &mut budget(4096, 1));
        assert!(result.is_err(), "accepted corruption {change}");
        assert!(
            !matches!(result, Err(StoreError::UnsupportedLocator(_))),
            "corruption must not be mistaken for valid foreign encoding"
        );
    }
}

#[test]
fn known_foreign_encoding_and_document_uri_are_explicitly_unsupported() {
    let store = located_store();
    #[cfg(unix)]
    let (encoding, raw, display) = (
        "windows_utf16_le",
        vec![b'C', 0, b':', 0, b'\\', 0, b'x', 0],
        "C:\\x",
    );
    #[cfg(windows)]
    let (encoding, raw, display) = ("unix_bytes", b"/tmp/x".to_vec(), "/tmp/x");
    #[cfg(any(unix, windows))]
    {
        store.connection.execute("UPDATE nodes SET native_locator_encoding=?1,native_locator_raw=?2,locator_key=?3 WHERE snapshot_id='located' AND id=2",params![encoding,raw,serde_json::to_string(&ResourceLocator::NativePath(display.into())).unwrap()]).unwrap();
        assert!(matches!(
            store.native_locator_bounded("located", 2, &mut budget(4096, 1)),
            Err(StoreError::UnsupportedLocator(_))
        ));
    }
    let uri = "content://documents/item";
    store.connection.execute("UPDATE nodes SET native_locator_kind='document_uri',native_locator_encoding='utf8_uri',native_locator_raw=?1,locator_key=?2 WHERE snapshot_id='located' AND id=2",params![uri.as_bytes(),serde_json::to_string(&ResourceLocator::DocumentUri(uri.into())).unwrap()]).unwrap();
    assert!(matches!(
        store.native_locator_bounded("located", 2, &mut budget(4096, 1)),
        Err(StoreError::UnsupportedLocator(_))
    ));
}

#[test]
fn staging_encoding_cost_matches_every_persisted_payload_field() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut node = graph("cost", 1).nodes.remove(1);
    node.name = "İ-文件-\"".into();
    node.locator = ResourceLocator::NativePath("/tmp/İ-文件-\"".into());
    let locator =
        QualifiedLocator::from_native_path(std::path::Path::new("/tmp/İ-文件-\"")).unwrap();
    let cost = staging_node_encoded_cost(&node, Some(&locator), Some(777)).unwrap();
    store
        .append_staging_located_iter("cost", std::iter::once((&node, &locator, Some(777))))
        .unwrap();
    let actual:i64=store.connection.query_row("SELECT length(CAST(s.node_json AS BLOB))+length(CAST(f.name_fold AS BLOB))+length(CAST(f.path_fold AS BLOB))+length(CAST(s.native_locator_kind AS BLOB))+length(CAST(s.native_locator_encoding AS BLOB))+length(s.native_locator_raw)+8 FROM scan_staging s JOIN scan_staging_search f ON f.job_id=s.job_id AND f.node_seq=s.node_seq WHERE s.job_id='cost'",[],|row|row.get(0)).unwrap();
    assert_eq!(cost, actual as u64);
    let mut invalid = locator.clone();
    invalid.display = "/wrong".into();
    assert!(
        store
            .append_staging_located_iter(
                "atomic",
                [(&node, &locator, None), (&node, &invalid, None)].into_iter()
            )
            .is_err()
    );
    assert_eq!(store.staging_node_count("atomic").unwrap(), 0);
}

#[test]
fn located_owned_publication_failure_keeps_staging_and_rolls_back_raw_rows() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = graph("rollback", 100);
    let locators: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| match &node.locator {
            ResourceLocator::NativePath(path) => {
                QualifiedLocator::from_native_path(std::path::Path::new(path)).unwrap()
            }
            _ => unreachable!(),
        })
        .collect();
    store
        .append_staging_located_iter(
            "job:2",
            graph
                .nodes
                .iter()
                .zip(&locators)
                .map(|(node, locator)| (node, locator, Some(11))),
        )
        .unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_locator_latest BEFORE INSERT ON latest_revision BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        store
            .publish_revision_owned("job:2", &graph, "failed", 1, Some(("s", "scope")))
            .is_err()
    );
    assert_eq!(
        store.staging_node_count("job:2").unwrap(),
        graph.nodes.len() as u64
    );
    assert!(
        store
            .native_locator_bounded("rollback", 1, &mut budget(4096, 1))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .latest_revision_for_root(&graph.snapshot.root)
            .unwrap()
            .is_none()
    );
    store.clear_stale_job_staging("job", 2).unwrap();
    assert_eq!(store.staging_node_count("job:2").unwrap(), 2);
    store.clear_stale_job_staging("job", 3).unwrap();
    assert_eq!(store.staging_node_count("job:2").unwrap(), 0);
}

#[cfg(unix)]
fn alias_graph() -> (diskgraph_core::DiskGraph, Vec<QualifiedLocator>) {
    use std::os::unix::ffi::OsStringExt;
    let mut graph = graph("aliases", 100);
    let first = QualifiedLocator::from_native_path(std::path::Path::new(
        &std::ffi::OsString::from_vec(b"/tmp/diskgraph-test/\xff".to_vec()),
    ))
    .unwrap();
    let second = QualifiedLocator::from_native_path(std::path::Path::new(
        &std::ffi::OsString::from_vec(b"/tmp/diskgraph-test/\xfe".to_vec()),
    ))
    .unwrap();
    assert_eq!(first.display(), second.display());
    let root =
        QualifiedLocator::from_native_path(std::path::Path::new("/tmp/diskgraph-test")).unwrap();
    graph.nodes[1].locator = ResourceLocator::NativePath(first.display().into());
    let mut other = graph.nodes[1].clone();
    other.id = 3;
    graph.nodes.push(other);
    (graph, vec![root, first, second])
}

#[cfg(unix)]
#[test]
fn same_display_distinct_raw_nodes_publish_only_with_complete_exact_staging() {
    let (graph, locators) = alias_graph();
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    assert!(store.save(&graph).is_err());
    assert!(store.publish_revision("trusted", &graph, "bad", 1).is_err());
    store
        .append_staging_located_iter(
            "aliases",
            graph
                .nodes
                .iter()
                .zip(&locators)
                .map(|(node, locator)| (node, locator, Some(44))),
        )
        .unwrap();
    store
        .publish_revision_owned("aliases", &graph, "good", 1, Some(("s", "scope")))
        .unwrap();
    let a = store
        .native_locator_bounded("aliases", 2, &mut budget(4096, 1))
        .unwrap()
        .unwrap()
        .locator
        .unwrap();
    let b = store
        .native_locator_bounded("aliases", 3, &mut budget(4096, 1))
        .unwrap()
        .unwrap()
        .locator
        .unwrap();
    assert_eq!(a.display(), b.display());
    assert_ne!(a.raw_bytes(), b.raw_bytes());
    assert_eq!(a.raw_bytes(), locators[1].raw_bytes());
    assert_eq!(b.raw_bytes(), locators[2].raw_bytes());
}

#[cfg(unix)]
#[test]
fn display_aliases_reject_old_staging_missing_identity_and_mismatched_exact_nodes() {
    for corruption in [
        "native_locator_raw=NULL",
        "native_locator_kind='document_uri'",
        "node_json=json_set(node_json,'$.id',999)",
        "node_json=json_set(node_json,'$.locator.value','/forged')",
        "native_locator_raw=(SELECT native_locator_raw FROM scan_staging WHERE job_id='aliases' AND node_seq=2)",
    ] {
        let (graph, locators) = alias_graph();
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        store
            .append_staging_located_iter(
                "aliases",
                graph
                    .nodes
                    .iter()
                    .zip(&locators)
                    .map(|(node, locator)| (node, locator, None)),
            )
            .unwrap();
        store
            .connection
            .execute(
                &format!(
                    "UPDATE scan_staging SET {corruption} WHERE job_id='aliases' AND node_seq=3"
                ),
                [],
            )
            .unwrap();
        assert!(
            store
                .publish_revision_owned("aliases", &graph, "bad", 1, Some(("s", "scope")))
                .is_err(),
            "accepted {corruption}"
        );
        assert_eq!(store.staging_node_count("aliases").unwrap(), 3);
        assert!(
            store
                .latest_revision_for_root(&graph.snapshot.root)
                .unwrap()
                .is_none()
        );
        assert!(store.snapshot("aliases").is_err());
    }
    let (graph, _) = alias_graph();
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.append_staging_nodes("old", &graph.nodes).unwrap();
    assert!(
        store
            .publish_revision_owned("old", &graph, "bad", 1, Some(("s", "scope")))
            .is_err()
    );
}

fn downgrade_v11_to_v10(connection: &rusqlite::Connection) {
    connection
        .execute_batch(
            "DROP TRIGGER revisions_require_locator_writer;
        ALTER TABLE graph_revisions DROP COLUMN locator_writer_generation;
        ALTER TABLE nodes DROP COLUMN native_locator_kind;
        ALTER TABLE nodes DROP COLUMN native_locator_encoding;
        ALTER TABLE nodes DROP COLUMN native_locator_raw;
        ALTER TABLE nodes DROP COLUMN self_modified_unix_seconds;
        ALTER TABLE scan_staging DROP COLUMN native_locator_kind;
        ALTER TABLE scan_staging DROP COLUMN native_locator_encoding;
        ALTER TABLE scan_staging DROP COLUMN native_locator_raw;
        ALTER TABLE scan_staging DROP COLUMN self_modified_unix_seconds;
        PRAGMA user_version=10;",
        )
        .unwrap();
}

#[test]
fn v10_migration_backups_preserve_unknown_locators_and_reject_already_open_old_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let mut store = SqliteSnapshotStore::open(&path).unwrap();
    store.save(&graph("legacy", 100)).unwrap();
    downgrade_v11_to_v10(&store.connection);
    let old = rusqlite::Connection::open(&path).unwrap();
    let (mut current, backup) =
        SqliteSnapshotStore::open_with_backup(&path, &dir.path().join("backups")).unwrap();
    let backup = rusqlite::Connection::open(backup.unwrap()).unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        10
    );
    assert_eq!(
        backup
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('nodes') WHERE name='native_locator_raw'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let legacy = current
        .native_locator_bounded("legacy", 2, &mut budget(4096, 1))
        .unwrap()
        .unwrap();
    assert!(legacy.locator.is_none());
    assert_eq!(legacy.self_modified, None);
    let refused=old.execute("INSERT INTO graph_revisions(revision_id,snapshot_id,published_at_unix_ms,writer_generation) VALUES ('obsolete','legacy',1,10)",[]).unwrap_err();
    assert!(refused.to_string().contains("obsolete locator writer"));
    current
        .publish_revision("current", &graph("current", 101), "current", 2)
        .unwrap();
    let generations:(i64,i64,i64)=current.connection.query_row("SELECT r.writer_generation,r.locator_writer_generation,s.count_schema FROM graph_revisions r JOIN snapshots s ON s.id=r.snapshot_id WHERE r.revision_id='current'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(generations, (10, 11, 9));
}

#[test]
fn native_locator_point_query_does_not_scan_or_decode_unrelated_nodes() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for unrelated in [20_000, 200_000] {
        let store = located_store();
        store.connection.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<?1)
        INSERT INTO nodes(snapshot_id,id,parent_id,locator_key,name,subtree_bytes,node_json,native_locator_kind,native_locator_encoding,native_locator_raw)
        SELECT 'located',x+2,1,'null','unrelated',0,'null','invalid','invalid',zeroblob(100) FROM n;", [unrelated]).unwrap();
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
            .native_locator_bounded("located", 2, &mut budget(4096, 1))
            .unwrap()
            .unwrap();
        store
            .connection
            .progress_handler(0, None::<fn() -> bool>)
            .unwrap();
        assert!(result.locator.is_some());
        let work = steps.load(Ordering::Relaxed);
        eprintln!("D30 native locator: unrelated={unrelated}, selected=1, sqlite_vm_steps={work}");
        assert!(work < 500);
    }
}
