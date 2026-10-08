//! 终检独立窄读：真实WAL旧快照、隔离拒权及原期限，不以旧消费者连接作确认。
use crate::{RevisionOwnershipReader, SqliteSnapshotStore, StoreError};
use std::time::{Duration, Instant};

#[test]
fn fresh_terminal_reader_rejects_quarantine_hidden_from_consumer_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let mut writer = SqliteSnapshotStore::open(&path).unwrap();
    let mut graph = crate::tests::graph("snapshot", 100);
    graph.evidence.clear();
    writer.append_staging_nodes("stage", &graph.nodes).unwrap();
    writer
        .publish_revision_owned("stage", &graph, "base", 1, Some(("server", "scope")))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let consumer = SqliteSnapshotStore::open_reader_until(&path, deadline, None).unwrap();
    consumer.connection.execute_batch("BEGIN").unwrap();
    assert!(
        consumer
            .revision_ownership_matches("base", "server", "scope")
            .unwrap()
    );
    assert_eq!(
        writer
            .isolate_unconfirmed_revision_roots("server", "scope", None)
            .unwrap(),
        1
    );
    assert!(
        consumer
            .revision_ownership_matches("base", "server", "scope")
            .unwrap(),
        "fixture must hold an actual stale WAL snapshot"
    );
    let terminal = RevisionOwnershipReader::open_until(&path, deadline).unwrap();
    assert!(!terminal.matches("base", "server", "scope").unwrap());
    assert!(!terminal.matches("base", "server", "foreign").unwrap());
    consumer.connection.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn expired_terminal_admission_does_not_open_or_create_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not_born.sqlite");
    assert!(matches!(
        RevisionOwnershipReader::open_until(&path, Instant::now()),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(!path.exists());
}

#[test]
fn terminal_sql_is_interrupted_under_original_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let writer = SqliteSnapshotStore::open(&path).unwrap();
    // 实际SQL强制执行百万步，不用sleep或模拟错误证明progress handler有效。
    writer.connection.execute_batch("DROP VIEW revision_authorized_ownership; CREATE VIEW revision_authorized_ownership AS WITH RECURSIVE work(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM work WHERE n<5000000) SELECT CAST(n AS TEXT) revision_id, 'server' server_id, 'scope' scope_id FROM work;").unwrap();
    let started = Instant::now();
    let terminal =
        RevisionOwnershipReader::open_until(&path, started + Duration::from_millis(50)).unwrap();
    let error = terminal.matches("never", "server", "scope").unwrap_err();
    assert!(
        matches!(error, StoreError::BudgetExceeded) || error.is_interrupted(),
        "unexpected SQL failure: {error}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
#[ignore = "release benchmark: compare actual independent terminal connection costs"]
fn release_terminal_connection_costs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let mut writer = SqliteSnapshotStore::open(&path).unwrap();
    let mut graph = crate::tests::graph("snapshot", 100);
    graph.evidence.clear();
    writer.append_staging_nodes("stage", &graph.nodes).unwrap();
    writer
        .publish_revision_owned("stage", &graph, "base", 1, Some(("server", "scope")))
        .unwrap();
    let mut general = Vec::new();
    let mut narrow = Vec::new();
    for index in 0..210 {
        for specialized in [index % 2 == 0, index % 2 != 0] {
            let started = Instant::now();
            let deadline = started + Duration::from_secs(1);
            if specialized {
                let reader = RevisionOwnershipReader::open_until(&path, deadline).unwrap();
                assert!(reader.matches("base", "server", "scope").unwrap());
            } else {
                let reader = SqliteSnapshotStore::open_reader_until(&path, deadline, None).unwrap();
                assert!(
                    reader
                        .revision_ownership_matches("base", "server", "scope")
                        .unwrap()
                );
            }
            if index >= 10 {
                if specialized {
                    narrow.push(started.elapsed().as_nanos());
                } else {
                    general.push(started.elapsed().as_nanos());
                }
            }
        }
    }
    general.sort_unstable();
    narrow.sort_unstable();
    eprintln!(
        "DG_TERMINAL_READ_COST samples=200 general_p50_ns={} general_p95_ns={} narrow_p50_ns={} narrow_p95_ns={}",
        general[100], general[190], narrow[100], narrow[190]
    );
}

#[test]
fn successive_terminal_selects_observe_intervening_wal_commit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let mut writer = SqliteSnapshotStore::open(&path).unwrap();
    let mut graph = crate::tests::graph("snapshot", 100);
    graph.evidence.clear();
    writer.append_staging_nodes("stage", &graph.nodes).unwrap();
    writer
        .publish_revision_owned("stage", &graph, "base", 1, Some(("server", "scope")))
        .unwrap();
    let terminal =
        RevisionOwnershipReader::open_until(&path, Instant::now() + Duration::from_secs(5))
            .unwrap();
    assert!(terminal.matches("base", "server", "scope").unwrap());
    assert_eq!(
        writer
            .isolate_unconfirmed_revision_roots("server", "scope", None)
            .unwrap(),
        1
    );
    assert!(
        !terminal.matches("base", "server", "scope").unwrap(),
        "terminal must not freeze its first SELECT in a transaction"
    );
}

#[test]
fn a_new_terminal_connection_observes_database_replaced_at_same_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    {
        let mut writer = SqliteSnapshotStore::open(&path).unwrap();
        let mut graph = crate::tests::graph("snapshot", 100);
        graph.evidence.clear();
        writer.append_staging_nodes("stage", &graph.nodes).unwrap();
        writer
            .publish_revision_owned("stage", &graph, "base", 1, Some(("server", "scope")))
            .unwrap();
    }
    {
        let terminal =
            RevisionOwnershipReader::open_until(&path, Instant::now() + Duration::from_secs(5))
                .unwrap();
        assert!(terminal.matches("base", "server", "scope").unwrap());
    }
    std::fs::rename(&path, directory.path().join("original.sqlite")).unwrap();
    // 替换后的真实数据库有相同 revision，但属于不同服务器与范围。
    {
        let mut replacement = SqliteSnapshotStore::open(&path).unwrap();
        let mut graph = crate::tests::graph("replacement", 100);
        graph.evidence.clear();
        replacement
            .append_staging_nodes("stage", &graph.nodes)
            .unwrap();
        replacement
            .publish_revision_owned(
                "stage",
                &graph,
                "base",
                1,
                Some(("foreign-server", "foreign-scope")),
            )
            .unwrap();
    }
    let terminal =
        RevisionOwnershipReader::open_until(&path, Instant::now() + Duration::from_secs(5))
            .unwrap();
    assert!(!terminal.matches("base", "server", "scope").unwrap());
    assert!(
        terminal
            .matches("base", "foreign-server", "foreign-scope")
            .unwrap()
    );
}

#[test]
fn missing_ownership_view_is_an_error_and_never_authorization() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE unrelated (id INTEGER)")
        .unwrap();
    let terminal =
        RevisionOwnershipReader::open_until(&path, Instant::now() + Duration::from_secs(5))
            .unwrap();
    assert!(terminal.matches("base", "server", "scope").is_err());
}

#[test]
fn a_later_busy_select_uses_only_the_original_remaining_window() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE ownership(revision_id TEXT,server_id TEXT,scope_id TEXT); CREATE VIEW revision_authorized_ownership AS SELECT * FROM ownership;").unwrap();
    let started = Instant::now();
    let deadline = started + Duration::from_millis(150);
    let terminal = RevisionOwnershipReader::open_until(&path, deadline).unwrap();
    let admission_elapsed = started.elapsed();
    // 真实 rollback-journal 写锁阻止新读；在原窗口已用去大半后才执行 SELECT。
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let locked_elapsed = started.elapsed();
    std::thread::sleep(Duration::from_millis(100));
    let select_started = Instant::now();
    let before_select_elapsed = started.elapsed();
    let result = terminal.matches("base", "server", "scope");
    let select_elapsed = select_started.elapsed();
    let total_elapsed = started.elapsed();
    // 原墙钟门禁不放宽；分开记录调度/夹具准备与真实 SELECT，避免将总耗时推断为重试续期。
    // 计时截至原查询返回；诊断输出自身不能增加被测操作的实耗。
    eprintln!(
        "terminal_busy_phases admission={admission_elapsed:?} locked={locked_elapsed:?} before_select={before_select_elapsed:?} select={select_elapsed:?} total={total_elapsed:?} result={result:?}"
    );
    assert!(
        matches!(&result, Err(StoreError::BudgetExceeded))
            || matches!(&result, Err(error) if error.is_busy() || error.is_interrupted()),
        "{result:?}"
    );
    assert!(
        total_elapsed < Duration::from_millis(240),
        "old busy timeout renewed the original window: {:?}",
        total_elapsed
    );
    connection.execute_batch("ROLLBACK").unwrap();
}
