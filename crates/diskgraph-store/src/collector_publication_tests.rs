//! EV-05 采集发布、来源约束和旧成员迁移的隔离回归。

use crate::tests::graph;
use crate::{SUPPORTED_SCHEMA_VERSION, SqliteSnapshotStore};
use diskgraph_core::{
    AssertionKind, CollectorBatch, CollectorRun, Entity, EntityKind, EvidenceRecord, Polarity,
    Relation, RelationEdge,
};
use rusqlite::params;

pub(super) fn batch(run: &str, snapshot: &str) -> CollectorBatch {
    CollectorBatch {
        run: CollectorRun {
            run_id: run.into(),
            snapshot_id: snapshot.into(),
            collector_id: "test".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: 1,
            coverage_complete: true,
            errors: vec![],
        },
        entities: vec![
            Entity {
                entity_id: "resource".into(),
                kind: EntityKind::Resource,
                identity: "{\"node_id\":2}".into(),
                display: "resource".into(),
                source_run_id: run.into(),
            },
            Entity {
                entity_id: format!("project-{run}"),
                kind: EntityKind::Project,
                identity: "{}".into(),
                display: "project".into(),
                source_run_id: run.into(),
            },
        ],
        evidence: vec![EvidenceRecord {
            evidence_id: format!("evidence-{run}"),
            run_id: run.into(),
            basis: "fixture".into(),
            observed_at_unix_ms: 1,
            expires_at_unix_ms: None,
            confidence: 90,
            input_fingerprint: "fixture".into(),
        }],
        edges: vec![RelationEdge {
            edge_id: format!("edge-{run}"),
            source_entity_id: "resource".into(),
            relation: Relation::OwnedByProject,
            target_entity_id: format!("project-{run}"),
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![(format!("evidence-{run}"), Polarity::Supports)],
        }],
    }
}

pub(super) fn base() -> SqliteSnapshotStore {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store
        .publish_revision("job", &graph("snap", 100), "base", 1)
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO revision_ownership VALUES ('base','server','scope')",
            [],
        )
        .unwrap();
    store
}

fn count(store: &SqliteSnapshotStore, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn collector_publication_keeps_revision_selection_and_reuses_immutable_entities() {
    let mut store = base();
    let first = batch("one", "snap");
    store
        .publish_collector_revision(
            "base",
            "r1",
            2,
            ("server", "scope"),
            &first,
            &[("one", "active")],
        )
        .unwrap();
    let mut second = batch("two", "snap");
    second.entities[0] = first.entities[0].clone();
    assert!(
        store
            .publish_collector_revision(
                "r1",
                "missing-upstream",
                3,
                ("server", "scope"),
                &second,
                &[("two", "active")]
            )
            .is_err()
    );
    store
        .publish_collector_revision(
            "r1",
            "r2",
            3,
            ("server", "scope"),
            &second,
            &[("two", "active"), ("one", "dependency_only")],
        )
        .unwrap();
    let role: String = store
        .connection
        .query_row(
            "SELECT role FROM revision_runs WHERE revision_id='r1' AND run_id='one'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(role, "active");
    assert_eq!(
        store.revision("r1").unwrap().snapshot_id,
        store.revision("r2").unwrap().snapshot_id
    );
    assert_eq!(count(&store, "snapshots"), 1);
    assert_eq!(count(&store, "entities"), 3);
    assert_eq!(count(&store, "entity_run_memberships"), 4);
    assert_eq!(count(&store, "relation_run_memberships"), 2);
    assert_eq!(
        store
            .latest_revision_for_root(&graph("snap", 100).snapshot.root)
            .unwrap()
            .as_deref(),
        Some("r2")
    );
}

#[test]
fn collector_publication_database_failure_rolls_back_every_layer() {
    let mut store = base();
    store.connection.execute_batch("CREATE TRIGGER fail_latest BEFORE UPDATE ON latest_revision BEGIN SELECT RAISE(ABORT,'injected latest failure'); END;").unwrap();
    assert!(
        store
            .publish_collector_revision(
                "base",
                "failed",
                2,
                ("server", "scope"),
                &batch("one", "snap"),
                &[("one", "active")]
            )
            .is_err()
    );
    for table in [
        "collector_runs",
        "entities",
        "evidence_records",
        "relations",
        "entity_run_memberships",
        "relation_run_memberships",
        "revision_runs",
    ] {
        assert_eq!(count(&store, table), 0, "{table}");
    }
    assert_eq!(count(&store, "graph_revisions"), 1);
    assert_eq!(count(&store, "revision_ownership"), 1);
    assert_eq!(
        store
            .latest_revision_for_root(&graph("snap", 100).snapshot.root)
            .unwrap()
            .as_deref(),
        Some("base")
    );
}

#[test]
fn collector_publication_rejects_owner_snapshot_role_and_forged_provenance() {
    let mut store = base();
    let valid = batch("one", "snap");
    assert!(
        store
            .publish_collector_revision(
                "base",
                "bad",
                2,
                ("wrong", "scope"),
                &valid,
                &[("one", "active")]
            )
            .is_err()
    );
    assert!(
        store
            .publish_collector_revision(
                "base",
                "bad",
                2,
                ("server", "scope"),
                &batch("one", "other"),
                &[("one", "active")]
            )
            .is_err()
    );
    assert!(
        store
            .publish_collector_revision(
                "base",
                "bad",
                2,
                ("server", "scope"),
                &valid,
                &[("one", "unknown")]
            )
            .is_err()
    );
    let mut forged = valid.clone();
    forged.evidence[0].run_id = "forged".into();
    assert!(
        store
            .publish_collector_revision(
                "base",
                "bad",
                2,
                ("server", "scope"),
                &forged,
                &[("one", "active")]
            )
            .is_err()
    );
    forged = valid.clone();
    forged.entities[0].source_run_id = "forged".into();
    assert!(
        store
            .publish_collector_revision(
                "base",
                "bad",
                2,
                ("server", "scope"),
                &forged,
                &[("one", "active")]
            )
            .is_err()
    );
    assert_eq!(count(&store, "collector_runs"), 0);
    store
        .publish_collector_revision(
            "base",
            "r1",
            2,
            ("server", "scope"),
            &valid,
            &[("one", "active")],
        )
        .unwrap();
    let mut conflict = batch("two", "snap");
    conflict.entities[0] = valid.entities[0].clone();
    conflict.entities[0].display = "changed".into();
    assert!(
        store
            .publish_collector_revision(
                "r1",
                "bad",
                3,
                ("server", "scope"),
                &conflict,
                &[("two", "active"), ("one", "dependency_only")]
            )
            .is_err()
    );
    assert_eq!(count(&store, "collector_runs"), 1);
}

#[test]
fn compatibility_binding_rejects_cross_snapshot_and_rolls_back_partial_selection() {
    let mut store = base();
    store.save(&graph("other", 200)).unwrap();
    for snapshot in ["snap", "other"] {
        let b = batch(snapshot, snapshot);
        store
            .record_collector_batch(snapshot, &b.run, &b.entities, &b.evidence, &b.edges)
            .unwrap();
    }
    assert!(
        store
            .bind_runs_to_revision("base", &[("snap", "active"), ("other", "active")])
            .is_err()
    );
    assert_eq!(count(&store, "revision_runs"), 0);
    assert!(
        store
            .bind_runs_to_revision("base", &[("snap", "garbage")])
            .is_err()
    );
    assert_eq!(count(&store, "revision_runs"), 0);
}

#[test]
fn migration_v9_preserves_only_unambiguous_membership() {
    let mut store = base();
    let b = batch("one", "snap");
    store
        .record_collector_batch("snap", &b.run, &b.entities, &b.evidence, &b.edges)
        .unwrap();
    let mut second = batch("two", "snap");
    second.entities[0] = b.entities[0].clone();
    store
        .record_collector_batch(
            "snap",
            &second.run,
            &second.entities,
            &second.evidence,
            &second.edges,
        )
        .unwrap();

    let mut ambiguous = b.edges[0].clone();
    ambiguous.edge_id = "ambiguous".into();
    ambiguous
        .evidence_refs
        .push((second.evidence[0].evidence_id.clone(), Polarity::Supports));
    store.connection.execute("INSERT INTO relations VALUES ('snap','ambiguous','resource','owned_by_project','project-one',?1)",[serde_json::to_string(&ambiguous).unwrap()]).unwrap();
    let mut forged = b.entities[0].clone();
    forged.entity_id = "forged".into();
    forged.source_run_id = "missing".into();
    store
        .connection
        .execute(
            "INSERT INTO entities VALUES ('snap','forged','resource',?1)",
            [serde_json::to_string(&forged).unwrap()],
        )
        .unwrap();
    store.connection.execute_batch("DROP TRIGGER revisions_require_native_observation_writer;
         ALTER TABLE graph_revisions DROP COLUMN native_observation_writer_generation;
         ALTER TABLE nodes DROP COLUMN native_observation_format;
         ALTER TABLE nodes DROP COLUMN native_observation_raw;
         ALTER TABLE nodes DROP COLUMN native_observation_gap;
         ALTER TABLE scan_staging DROP COLUMN native_observation_format;
         ALTER TABLE scan_staging DROP COLUMN native_observation_raw;
         ALTER TABLE scan_staging DROP COLUMN native_observation_gap;
         DROP TRIGGER revisions_require_locator_writer;
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
    store
        .connection
        .execute(
            "INSERT INTO revision_runs VALUES ('base','one','active')",
            [],
        )
        .unwrap();
    let store = SqliteSnapshotStore::initialize(store.connection).unwrap();
    assert_eq!(SUPPORTED_SCHEMA_VERSION, 13);
    assert_eq!(count(&store, "relation_run_memberships"), 2);
    assert_eq!(count(&store, "entity_run_memberships"), 3);
    assert_eq!(count(&store, "collector_membership_diagnostics"), 4);
    assert_eq!(count(&store, "revision_runs"), 1);
    let membership: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM relation_run_memberships WHERE edge_id=?1",
            params!["ambiguous"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(membership, 0);
    // 原始可信快照 API 保留旧数据，不将迁移缺失伪装成删除。
    assert_eq!(store.edges_from("snap", "resource", None).unwrap().len(), 3);
}

#[test]
fn scan_publication_includes_batch_or_rolls_back_snapshot_and_staging() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = graph("snap", 100);
    let b = batch("one", "snap");
    store.append_staging_nodes("job", &graph.nodes).unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_binding BEFORE INSERT ON latest_revision BEGIN SELECT RAISE(ABORT,'injected binding failure'); END;").unwrap();
    assert!(
        store
            .publish_revision_owned_with_batch(
                "job",
                &graph,
                "r1",
                1,
                Some(("server", "scope")),
                Some(&b)
            )
            .is_err()
    );
    for table in [
        "snapshots",
        "nodes",
        "collector_runs",
        "entities",
        "evidence_records",
        "relations",
        "entity_run_memberships",
        "relation_run_memberships",
        "graph_revisions",
        "latest_revision",
    ] {
        assert_eq!(count(&store, table), 0, "{table}");
    }
    assert_eq!(
        store.staging_node_count("job").unwrap(),
        graph.nodes.len() as u64
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_binding;")
        .unwrap();
    store
        .publish_revision_owned_with_batch(
            "job",
            &graph,
            "r1",
            1,
            Some(("server", "scope")),
            Some(&b),
        )
        .unwrap();
    assert_eq!(store.staging_node_count("job").unwrap(), 0);
    assert_eq!(count(&store, "revision_ownership"), 1);
    assert_eq!(count(&store, "snapshots"), 1);
    assert_eq!(count(&store, "revision_runs"), 1);
    assert_eq!(count(&store, "relation_run_memberships"), 1);
}
