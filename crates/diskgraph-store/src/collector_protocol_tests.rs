//! 升级中的旧写入进程、封存选择与重新采集恢复回归。
use crate::SqliteSnapshotStore;
use crate::collector_publication_tests::{base, batch};

#[test]
fn sealed_revision_rejects_late_batch_append() {
    let mut store = base();
    let batch = batch("late", "snap");
    store
        .record_collector_batch(
            "snap",
            &batch.run,
            &batch.entities,
            &batch.evidence,
            &batch.edges,
        )
        .unwrap();
    assert!(
        store
            .bind_runs_to_revision("base", &[("late", "active")])
            .is_err(),
        "published revision accepted a new batch"
    );
    assert!(
        store
            .revision_evidence("base")
            .unwrap()
            .all_edges()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn recollection_recovers_new_revision_without_rewriting_ambiguous_history() {
    let store = base();
    store.connection.execute("INSERT INTO relations VALUES ('snap','legacy','resource','owned_by_project','project','{}')",[]).unwrap();
    downgrade_to_v9(&store.connection);
    let mut store = SqliteSnapshotStore::initialize(store.connection).unwrap();
    assert!(
        store
            .revision_evidence("base")
            .unwrap()
            .has_incomplete_membership()
            .unwrap()
    );
    let batch = batch("fresh", "snap");
    store
        .publish_collector_revision(
            "base",
            "recovered",
            2,
            ("server", "scope"),
            &batch,
            &[("fresh", "active")],
        )
        .unwrap();
    let reader = store.revision_evidence("recovered").unwrap();
    assert!(
        !reader.has_incomplete_membership().unwrap(),
        "confirmed new revision inherits unrelated legacy diagnostics"
    );
    assert_eq!(reader.all_edges().unwrap().len(), 1);
    assert!(
        store
            .revision_evidence("base")
            .unwrap()
            .has_incomplete_membership()
            .unwrap()
    );
}

#[test]
fn existing_v9_writer_cannot_publish_after_v10_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    drop(SqliteSnapshotStore::open(&path).unwrap());
    let old = rusqlite::Connection::open(&path).unwrap();
    downgrade_to_v9(&old);
    // 同一旧连接执行v9真实写入语句；升级前允许，升级后必须拒绝。
    let write = "INSERT INTO collector_runs(run_id,snapshot_id,collector_id,collector_version,rule_version,observed_at_unix_ms,coverage_complete,run_json) VALUES ('old-run','snap','fixture',1,1,1,1,'{}')";
    old.execute_batch("BEGIN").unwrap();
    old.execute(write, []).unwrap();
    old.execute_batch("ROLLBACK").unwrap();
    let (_upgraded, backup) =
        SqliteSnapshotStore::open_with_backup(&path, &dir.path().join("backups")).unwrap();
    assert!(backup.is_some());
    assert!(
        old.execute(write, []).is_err(),
        "already-open v9 collector wrote without membership protocol"
    );
}

#[test]
fn selected_existing_runs_require_their_original_entity_sources() {
    let mut store = base();
    let a = batch("a", "snap");
    store
        .publish_collector_revision("base", "a", 2, ("server", "scope"), &a, &[("a", "active")])
        .unwrap();
    let mut b = batch("b", "snap");
    b.entities[0] = a.entities[0].clone();
    store
        .publish_collector_revision(
            "a",
            "b",
            3,
            ("server", "scope"),
            &b,
            &[("b", "active"), ("a", "dependency_only")],
        )
        .unwrap();
    let mut c = batch("c", "snap");
    c.entities.clear();
    c.edges.clear();
    c.evidence.clear();
    assert!(
        store
            .publish_collector_revision(
                "b",
                "c",
                4,
                ("server", "scope"),
                &c,
                &[("c", "active"), ("b", "active")]
            )
            .is_err(),
        "selected existing run omitted its original source"
    );
    store
        .publish_collector_revision(
            "b",
            "c",
            4,
            ("server", "scope"),
            &c,
            &[("c", "active"), ("b", "active"), ("a", "dependency_only")],
        )
        .unwrap();
}

#[test]
fn stale_collector_publication_cannot_replace_a_newer_file_scan() {
    let mut store = base();
    store
        .publish_revision(
            "new-scan",
            &crate::tests::graph("new-snapshot", 200),
            "new-scan",
            2,
        )
        .unwrap();
    let batch = batch("late", "snap");
    assert!(
        store
            .publish_collector_revision(
                "base",
                "late",
                3,
                ("server", "scope"),
                &batch,
                &[("late", "active")]
            )
            .is_err(),
        "late collector moved latest back to an old file tree"
    );
    assert_eq!(
        store
            .latest_revision_for_root(&crate::tests::graph("snap", 100).snapshot.root)
            .unwrap()
            .as_deref(),
        Some("new-scan")
    );
}

#[test]
fn occupied_resources_require_an_existing_snapshot_node() {
    for identity in ["{}", "{\"node_id\":99999}", "{\"node_id\":-1}"] {
        let mut store = base();
        let mut batch = batch("invalid", "snap");
        batch.entities[0].identity = identity.into();
        batch.entities[1].kind = diskgraph_core::EntityKind::Process;
        batch.edges[0].relation = diskgraph_core::Relation::UsedByProcess;
        assert!(
            store
                .publish_collector_revision(
                    "base",
                    "invalid",
                    2,
                    ("server", "scope"),
                    &batch,
                    &[("invalid", "active")]
                )
                .is_err(),
            "accepted unmappable occupancy {identity}"
        );
    }
}

pub(super) fn downgrade_to_v9(connection: &rusqlite::Connection) {
    connection.execute_batch("DROP TRIGGER revisions_require_locator_writer;
         ALTER TABLE graph_revisions DROP COLUMN locator_writer_generation;
         ALTER TABLE nodes DROP COLUMN native_locator_kind;
         ALTER TABLE nodes DROP COLUMN native_locator_encoding;
         ALTER TABLE nodes DROP COLUMN native_locator_raw;
         ALTER TABLE nodes DROP COLUMN self_modified_unix_seconds;
         ALTER TABLE scan_staging DROP COLUMN native_locator_kind;
         ALTER TABLE scan_staging DROP COLUMN native_locator_encoding;
         ALTER TABLE scan_staging DROP COLUMN native_locator_raw;
         ALTER TABLE scan_staging DROP COLUMN self_modified_unix_seconds;
         DROP TRIGGER revisions_require_collector_writer; DROP TRIGGER collectors_require_member_writer;
         DROP TRIGGER revisions_preserve_seal; DROP TRIGGER selected_runs_no_append;
         DROP TRIGGER selected_runs_no_rewrite; DROP TRIGGER selected_runs_no_remove;
         ALTER TABLE graph_revisions DROP COLUMN writer_generation;
         ALTER TABLE graph_revisions DROP COLUMN selection_sealed;
         ALTER TABLE graph_revisions DROP COLUMN evidence_complete;
         ALTER TABLE collector_runs DROP COLUMN writer_generation;
         DROP TRIGGER relation_membership_retarget; DROP TABLE relation_membership_adjacency; DROP TABLE relation_run_memberships; DROP TABLE entity_run_memberships; DROP TABLE collector_membership_diagnostics; PRAGMA user_version=9;").unwrap();
}

#[test]
fn selecting_incomplete_legacy_run_cannot_claim_complete() {
    let mut store = base();
    let a = batch("a", "snap");
    store
        .publish_collector_revision("base", "a", 2, ("server", "scope"), &a, &[("a", "active")])
        .unwrap();
    store.connection.execute("INSERT INTO relations VALUES ('snap','ambiguous','resource','owned_by_project','project-a','{}')",[]).unwrap();
    downgrade_to_v9(&store.connection);
    let mut store = SqliteSnapshotStore::initialize(store.connection).unwrap();
    let mut c = batch("c", "snap");
    c.entities.clear();
    c.edges.clear();
    c.evidence.clear();
    assert!(
        store
            .publish_collector_revision(
                "a",
                "c",
                3,
                ("server", "scope"),
                &c,
                &[("c", "active"), ("a", "active")]
            )
            .is_err()
    );
    store
        .publish_collector_revision("a", "c", 3, ("server", "scope"), &c, &[("c", "active")])
        .unwrap();
    assert!(
        !store
            .revision_evidence("c")
            .unwrap()
            .has_incomplete_membership()
            .unwrap()
    );
    assert!(
        store
            .revision_evidence("a")
            .unwrap()
            .has_incomplete_membership()
            .unwrap()
    );
}

#[test]
fn sealed_selection_rejects_raw_mutations_and_allows_parent_prune() {
    let mut store = base();
    let a = batch("a", "snap");
    store
        .publish_collector_revision("base", "a", 2, ("server", "scope"), &a, &[("a", "active")])
        .unwrap();
    for sql in [
        "UPDATE graph_revisions SET selection_sealed=0 WHERE revision_id='a'",
        "UPDATE graph_revisions SET evidence_complete=0 WHERE revision_id='a'",
        "UPDATE revision_runs SET role='dependency_only' WHERE revision_id='a'",
        "DELETE FROM revision_runs WHERE revision_id='a'",
        "INSERT INTO revision_runs VALUES ('base','a','active')",
    ] {
        assert!(
            store.connection.execute(sql, []).is_err(),
            "sealed mutation accepted: {sql}"
        );
    }
    let mut b = batch("b", "snap");
    b.entities[0] = a.entities[0].clone();
    store
        .publish_collector_revision(
            "a",
            "b",
            3,
            ("server", "scope"),
            &b,
            &[("b", "active"), ("a", "dependency_only")],
        )
        .unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_prune BEFORE DELETE ON graph_revisions WHEN OLD.revision_id='a' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.prune_revisions("server", "scope", 1, true).is_err());
    assert_eq!(
        store
            .revision_evidence("a")
            .unwrap()
            .all_edges()
            .unwrap()
            .len(),
        1
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_prune")
        .unwrap();
    assert_eq!(
        store
            .prune_revisions("server", "scope", 1, true)
            .unwrap()
            .len(),
        2
    );
    assert!(store.revision("a").is_err());
    assert_eq!(
        store
            .revision_evidence("b")
            .unwrap()
            .all_edges()
            .unwrap()
            .len(),
        1
    );
}
