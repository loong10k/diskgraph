//! 同轮编译复用只减少准备工作，不复用授权结果或 WAL 快照。
use super::RevisionOwnershipReader;
use crate::SqliteSnapshotStore;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

#[test]
fn repeated_ownership_sql_reuses_compilation_and_observes_committed_quarantine() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let mut writer = SqliteSnapshotStore::open(&path).unwrap();
    let mut graph = crate::tests::graph("snapshot", 100);
    graph.evidence.clear();
    writer.append_staging_nodes("stage", &graph.nodes).unwrap();
    writer
        .publish_revision_owned("stage", &graph, "base", 1, Some(("server", "scope")))
        .unwrap();
    let reader =
        RevisionOwnershipReader::open_until(&path, Instant::now() + Duration::from_secs(5))
            .unwrap();
    let prepares = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&prepares);
    reader
        .connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(context.action, AuthAction::Select) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Authorization::Allow
        }))
        .unwrap();
    assert!(reader.matches("base", "server", "scope").unwrap());
    let first = prepares.load(Ordering::Relaxed);
    assert!(first > 0, "must observe actual SQLite compilation");
    assert!(!reader.matches("base", "server", "foreign").unwrap());
    assert!(!reader.matches("missing", "server", "scope").unwrap());
    assert_eq!(
        writer
            .isolate_unconfirmed_revision_roots("server", "scope", None)
            .unwrap(),
        1
    );
    assert!(!reader.matches("base", "server", "scope").unwrap());
    assert_eq!(
        prepares.load(Ordering::Relaxed),
        first,
        "same-round queries recompile unchanged SQL"
    );
}

#[test]
fn compiled_ownership_sql_never_ignores_a_removed_authorization_view() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let writer = SqliteSnapshotStore::open(&path).unwrap();
    let reader =
        RevisionOwnershipReader::open_until(&path, Instant::now() + Duration::from_secs(5))
            .unwrap();
    assert!(!reader.matches("missing", "server", "scope").unwrap());
    writer
        .connection
        .execute_batch("DROP VIEW revision_authorized_ownership")
        .unwrap();
    assert!(reader.matches("missing", "server", "scope").is_err());
}
