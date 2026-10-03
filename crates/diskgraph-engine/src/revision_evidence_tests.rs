//! 经真实授权入口验证共享文件快照的采集批次隔离；来源：D28 / EV-05。
use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{
    AssertionKind, BusinessError, CollectorRun, Entity, EntityKind, EvidenceRecord, Polarity,
    PrincipalId, Relation, RelationEdge,
};
use diskgraph_store::SqliteSnapshotStore;

fn fixture() -> (tempfile::TempDir, Engine, PrincipalId, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"data").unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("evidence-reader").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let scope = engine.register_scope(&root, &actor, &policy).unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "evidence-reader").unwrap();
    let before = engine.latest_revision(&scope).unwrap().unwrap();
    let snapshot = engine
        .revision_reader()
        .unwrap()
        .revision(&before)
        .unwrap()
        .snapshot_id;
    let mut store = SqliteSnapshotStore::open(&dir.path().join("data/diskgraph.sqlite")).unwrap();
    let owner = store.revision_ownership(&before).unwrap().unwrap();
    let old = record(&snapshot, "old");
    store
        .publish_collector_revision(
            &before,
            "old-revision",
            2,
            (&owner.0, &owner.1),
            &old,
            &[("old", "active")],
        )
        .unwrap();
    let new = record(&snapshot, "new");
    store
        .publish_collector_revision(
            "old-revision",
            "new-revision",
            3,
            (&owner.0, &owner.1),
            &new,
            &[("new", "active"), ("old", "dependency_only")],
        )
        .unwrap();
    (
        dir,
        engine,
        actor,
        "old-revision".into(),
        "new-revision".into(),
    )
}

fn record(snapshot: &str, id: &str) -> diskgraph_core::CollectorBatch {
    let run = CollectorRun {
        run_id: id.into(),
        snapshot_id: snapshot.into(),
        collector_id: "process-fixture".into(),
        collector_version: 1,
        rule_version: 1,
        observed_at_unix_ms: 1,
        coverage_complete: false,
        errors: vec![],
    };
    let entities = [
        Entity {
            entity_id: format!("{id}-resource"),
            kind: EntityKind::Resource,
            identity: r#"{"node_id":2}"#.into(),
            display: "file".into(),
            source_run_id: id.into(),
        },
        Entity {
            entity_id: format!("{id}-process"),
            kind: EntityKind::Process,
            identity: format!("observation:{id}"),
            display: "fixture".into(),
            source_run_id: id.into(),
        },
    ];
    let evidence = [EvidenceRecord {
        evidence_id: format!("{id}-evidence"),
        run_id: id.into(),
        basis: "observed handle".into(),
        observed_at_unix_ms: 1,
        expires_at_unix_ms: Some(100),
        confidence: 100,
        input_fingerprint: id.into(),
    }];
    let edges = [RelationEdge {
        edge_id: format!("{id}-edge"),
        source_entity_id: format!("{id}-resource"),
        relation: Relation::UsedByProcess,
        target_entity_id: format!("{id}-process"),
        assertion_kind: AssertionKind::Observed,
        evidence_refs: vec![(format!("{id}-evidence"), Polarity::Supports)],
    }];
    diskgraph_core::CollectorBatch {
        run,
        entities: entities.into(),
        evidence: evidence.into(),
        edges: edges.into(),
    }
}

#[test]
fn old_revision_cannot_read_newer_batch_from_the_same_snapshot() {
    let (_dir, engine, actor, before, after) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    let old = engine
        .related_bounded(
            &before,
            "new-resource",
            None,
            true,
            None,
            10,
            &actor,
            &policy,
        )
        .unwrap();
    assert!(
        old["edges"].as_array().unwrap().is_empty(),
        "old revision leaked newer batch: {old}"
    );
    let new = engine
        .related_bounded(
            &after,
            "new-resource",
            None,
            true,
            None,
            10,
            &actor,
            &policy,
        )
        .unwrap();
    assert_eq!(new["edges"].as_array().unwrap().len(), 1);
}

#[test]
fn dependency_batches_do_not_reactivate_old_assertions() {
    let (_dir, engine, actor, before, after) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    let old = engine
        .related_bounded(
            &before,
            "old-resource",
            None,
            true,
            None,
            10,
            &actor,
            &policy,
        )
        .unwrap();
    assert_eq!(old["edges"].as_array().unwrap().len(), 1);
    let new = engine
        .related_bounded(
            &after,
            "old-resource",
            None,
            true,
            None,
            10,
            &actor,
            &policy,
        )
        .unwrap();
    assert!(
        new["edges"].as_array().unwrap().is_empty(),
        "dependency-only assertions became active: {new}"
    );
}

#[test]
fn old_revision_cannot_explain_newer_entities() {
    let (_dir, engine, actor, before, after) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    assert!(matches!(
        engine.explain_bounded(&before, "new-resource", None, 10, &actor, &policy),
        Err(EngineError::Business(BusinessError::NotFound))
    ));
    let new = engine
        .explain_bounded(&after, "new-resource", None, 10, &actor, &policy)
        .unwrap();
    assert_eq!(new["evidence"].as_array().unwrap().len(), 1);
}

#[test]
fn ambiguous_legacy_evidence_refuses_claims_but_preserves_file_tree() {
    let (dir, engine, actor, before, _after) = fixture();
    let reader = engine.revision_reader().unwrap();
    let snapshot = reader.revision(&before).unwrap().snapshot_id;
    let (_, scope) = reader.revision_ownership(&before).unwrap().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute("INSERT INTO graph_revisions(revision_id,snapshot_id,published_at_unix_ms,writer_generation,locator_writer_generation) VALUES ('ambiguous',?1,4,10,11)",[&snapshot]).unwrap();
    db.execute("INSERT INTO revision_ownership SELECT 'ambiguous',server_id,scope_id FROM revision_ownership WHERE revision_id=?1",[&before]).unwrap();
    db.execute("UPDATE graph_revisions SET selection_sealed=1,evidence_complete=0 WHERE revision_id='ambiguous'",[]).unwrap();
    let before = "ambiguous".to_owned();
    let policy = engine.policy_authorizer().unwrap();
    let error = engine
        .related_bounded(
            &before,
            "old-resource",
            None,
            true,
            None,
            10,
            &actor,
            &policy,
        )
        .unwrap_err();
    assert!(error.to_string().contains("recollect"), "{error}");
    let error = engine
        .review_candidates(
            &before,
            1,
            diskgraph_core::QueryBudget::default(),
            &actor,
            &policy,
        )
        .unwrap_err();
    assert!(error.to_string().contains("recollect"), "{error}");
    let budget = diskgraph_core::QueryBudget::default();
    engine
        .tree_view_until(
            &diskgraph_core::ScopeId::new(scope).unwrap(),
            &before,
            &actor,
            &policy,
            2,
            0,
            budget,
            diskgraph_core::query_deadline(budget).unwrap(),
        )
        .unwrap();
}
