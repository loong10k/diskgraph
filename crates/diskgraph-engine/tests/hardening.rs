use diskgraph_core::{
    AssertionKind, Permission, PolicyAuthorizer, PrincipalId, QueryBudget, Relation, RelationEdge,
    TruncationReason, Watermark,
};
use diskgraph_engine::content::{ConservativeProbe, InspectionRequest};
use diskgraph_engine::{Engine, EngineConfig};

fn setup() -> (
    tempfile::TempDir,
    Engine,
    PrincipalId,
    diskgraph_core::ScopeId,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), vec![1_u8; 1024]).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("hardening").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    (dir, engine, principal, scope)
}

#[test]
fn a_fresh_data_directory_accepts_work_when_its_volume_has_headroom() {
    let (_dir, engine, _principal, _scope) = setup();
    assert!(engine.accepts_new_work());
}

#[test]
fn digest_never_exceeds_its_budget_or_confirms_a_partial_file() {
    let (dir, engine, principal, scope) = setup();
    engine.set_content_read(&scope, &principal, true).unwrap();
    let path = dir.path().join("root/file");
    let request = InspectionRequest {
        scope_id: &scope,
        principal: &principal,
        path: &path,
        offset: 0,
        max_bytes: 1,
        chunk_bytes: 64,
        cancel: None,
    };
    let result = engine
        .digest_bounded(
            &request,
            &ConservativeProbe,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert!(result.bytes_digested <= 1);
    assert!(!result.confirmed());
    assert!(result.digest_hex.is_empty());
}

#[test]
fn a_scope_grant_cannot_authorize_another_scopes_revision() {
    let (dir, engine, principal, scope) = setup();
    let other = dir.path().join("other");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("private"), "private").unwrap();
    let other_scope = engine
        .register_scope(&other, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(
            &other_scope,
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    engine.run_job(&job.job_id, "worker").unwrap();
    let revision = engine.latest_revision(&other_scope).unwrap().unwrap();
    let mut authorizer = PolicyAuthorizer::new(1);
    authorizer.grant(principal.clone(), Permission::MetadataRead, scope.clone());
    assert!(
        engine
            .tree_view(&scope, &revision, &principal, &authorizer, 2, 0)
            .is_err()
    );
}

#[test]
fn high_degree_impact_reads_a_page_and_revocation_blocks_the_next_request() {
    let (dir, engine, principal, scope) = setup();
    let authorizer = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
    engine.run_job(&job.job_id, "impact-fixture").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let reader = engine.revision_reader().unwrap();
    let snapshot_id = reader.revision(&revision).unwrap().snapshot_id;
    drop(reader);
    let mut database =
        rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    let transaction = database.transaction().unwrap();
    for index in 0..1_000 {
        let edge = RelationEdge {
            edge_id: format!("impact-{index:04}"),
            source_entity_id: "impact-start".into(),
            relation: Relation::RebuildableBy,
            target_entity_id: format!("impact-target-{index:04}"),
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![],
        };
        transaction
            .execute(
                "INSERT INTO relations VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    snapshot_id,
                    edge.edge_id,
                    edge.source_entity_id,
                    edge.relation.wire_name(),
                    edge.target_entity_id,
                    serde_json::to_string(&edge).unwrap()
                ],
            )
            .unwrap();
    }
    transaction
        .execute(
            "UPDATE relations SET edge_json = '{bad-json' WHERE edge_id = 'impact-0999'",
            [],
        )
        .unwrap();
    transaction.commit().unwrap();

    let budget = QueryBudget {
        max_nodes: 10,
        max_edges: 10,
        ..QueryBudget::default()
    };
    let answer = engine
        .revision_impact(&revision, "impact-start", budget, &principal, &authorizer)
        .unwrap();
    assert_eq!(answer.entries.len(), 10);
    assert_eq!(answer.truncated, Some(TruncationReason::EdgeLimit));

    engine
        .revoke_scope(&scope, &principal, &authorizer)
        .unwrap();
    assert!(
        engine
            .revision_impact(&revision, "impact-start", budget, &principal, &authorizer,)
            .is_err()
    );
}

#[test]
fn a_revoked_queued_scan_never_publishes() {
    let (_dir, engine, principal, scope) = setup();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine
        .revoke_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    assert!(engine.run_job(&job.job_id, "worker").is_err());
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}

#[test]
fn capacity_refusal_prevents_job_creation() {
    let (dir, old, principal, scope) = setup();
    drop(old);
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        capacity_watermark: Watermark {
            warn_above_bytes: 1,
            refuse_above_bytes: 2,
        },
        ..EngineConfig::default()
    })
    .unwrap();
    assert!(!engine.accepts_new_work());
    assert!(
        engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .is_err()
    );
}

#[test]
fn an_in_place_write_invalidates_the_digest() {
    struct ChangingProbe;
    impl diskgraph_engine::content::PlaceholderProbe for ChangingProbe {
        fn is_placeholder(&self, path: &std::path::Path) -> bool {
            std::fs::write(path, vec![b'b'; 1024]).unwrap();
            false
        }
    }
    let (dir, engine, principal, scope) = setup();
    engine.set_content_read(&scope, &principal, true).unwrap();
    let path = dir.path().join("root/file");
    let request = InspectionRequest {
        scope_id: &scope,
        principal: &principal,
        path: &path,
        offset: 0,
        max_bytes: 1024,
        cancel: None,
        chunk_bytes: usize::MAX,
    };
    let outcome = engine.digest_bounded(
        &request,
        &ChangingProbe,
        &engine.policy_authorizer().unwrap(),
    );
    assert!(outcome.is_err() || !outcome.unwrap().confirmed());
}

#[test]
fn large_file_capacity_does_not_consume_metadata_staging_budget() {
    let (dir, _engine, principal, scope) = setup();
    std::fs::File::create(dir.path().join("root/huge"))
        .unwrap()
        .set_len(4 << 30)
        .unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        scan_budget: diskgraph_core::ScanBudget {
            max_staging_bytes: 64 * 1024,
            ..diskgraph_core::ScanBudget::default()
        },
        scan_options: diskgraph_disktree_core::scan::ScanOptions {
            apparent_size: true,
            ..Default::default()
        },
        ..EngineConfig::default()
    })
    .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "metadata-budget").unwrap();
    assert!(engine.latest_revision(&scope).unwrap().is_some());
}

#[test]
fn a_wide_tree_reports_its_node_budget() {
    let (dir, engine, principal, scope) = setup();
    for index in 0..200 {
        std::fs::write(dir.path().join(format!("root/file-{index}")), [0]).unwrap();
    }
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "wide-tree").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let view = engine
        .tree_view(
            &scope,
            &revision,
            &principal,
            &engine.policy_authorizer().unwrap(),
            2,
            0,
        )
        .unwrap();
    assert_eq!(view.root["truncation_reason"], "node_limit");
    assert!(view.root["nodes_read"].as_u64().unwrap() <= 100);
    for maximum in [4, 16] {
        let limited = engine
            .tree_view_bounded(
                &revision,
                2,
                0,
                diskgraph_core::QueryBudget {
                    max_nodes: maximum,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(limited.root["nodes_read"], maximum);
    }
}

#[test]
fn a_bounded_history_report_does_not_claim_complete_statistics() {
    let (dir, engine, principal, scope) = setup();
    for index in 0..200 {
        std::fs::write(dir.path().join(format!("root/file-{index}")), [0]).unwrap();
    }
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "history-budget").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let report = engine.compare_revisions(&revision, &revision, 2).unwrap();
    let json = report.to_json(None);
    assert_eq!(json["complete"], false);
    assert!(json["entries"].as_u64().unwrap() <= 100);
}

#[test]
fn pruning_retains_pins_and_the_latest_revision() {
    let (dir, engine, principal, scope) = setup();
    let mut revisions = Vec::new();
    for index in 0..3 {
        std::fs::write(dir.path().join(format!("root/new-{index}")), [0]).unwrap();
        let job = engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        engine.run_job(&job.job_id, "fixture").unwrap();
        revisions.push(engine.latest_revision(&scope).unwrap().unwrap());
    }
    let snapshot = engine.revision_snapshot(&revisions[0]).unwrap();
    engine
        .pin_snapshot(
            &snapshot.id,
            true,
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let removed = engine
        .prune_snapshots(
            &scope,
            1,
            true,
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].revision_id, revisions[1]);
    assert!(engine.revision_snapshot(&revisions[0]).is_ok());
    assert!(engine.revision_snapshot(&revisions[2]).is_ok());
}

#[cfg(unix)]
#[test]
fn ambiguous_legacy_revision_ownership_remains_denied() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root_a = dir.path().join("r�");
    let root_b = dir
        .path()
        .canonicalize()
        .unwrap()
        .join(std::ffi::OsString::from_vec(vec![b'r', 0xff]));
    std::fs::create_dir(&root_a).unwrap();
    std::fs::write(root_a.join("file"), [0]).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("legacy-admin").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope_a = engine
        .register_scope(&root_a, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope_a, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "fixture").unwrap();
    let indexed = engine.latest_revision(&scope_a).unwrap().unwrap();
    let mut legacy = engine.load_revision(&indexed).unwrap();
    legacy.snapshot.id = "legacy-snapshot".into();
    // 模拟另一系统保存的原始非 UTF-8 scope，其 v1 显示根与 Unicode 根相同。
    engine
        .control_store()
        .unwrap()
        .register_scope(&diskgraph_core::Locator::from_native_path(&root_b), None)
        .unwrap();
    drop(engine);
    let mut db =
        diskgraph_store::SqliteSnapshotStore::open(&data.join("diskgraph.sqlite")).unwrap();
    let revision = "legacy-unbound";
    db.publish_revision("legacy-job", &legacy, revision, 1)
        .unwrap();
    drop(db);

    let engine = Engine::open(EngineConfig {
        data_dir: data,
        ..EngineConfig::default()
    })
    .unwrap();
    assert!(
        engine
            .authorize_revision(
                Some(&scope_a),
                revision,
                &principal,
                &engine.policy_authorizer().unwrap()
            )
            .is_err()
    );
}

struct RevokingProbe<'a> {
    engine: &'a Engine,
    principal: &'a PrincipalId,
    scope: &'a diskgraph_core::ScopeId,
}
impl diskgraph_engine::content::PlaceholderProbe for RevokingProbe<'_> {
    fn is_placeholder(&self, _: &std::path::Path) -> bool {
        self.engine
            .set_content_read(self.scope, self.principal, false)
            .unwrap();
        false
    }
}

#[test]
fn withdrawing_content_authorization_after_preflight_prevents_hashing() {
    let (dir, engine, principal, scope) = setup();
    engine.set_content_read(&scope, &principal, true).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let path = dir.path().join("root/file");
    let request = InspectionRequest {
        scope_id: &scope,
        principal: &principal,
        path: &path,
        offset: 0,
        max_bytes: 1024,
        chunk_bytes: 64,
        cancel: None,
    };
    assert!(
        engine
            .digest_bounded(
                &request,
                &RevokingProbe {
                    engine: &engine,
                    principal: &principal,
                    scope: &scope
                },
                &policy
            )
            .is_err()
    );
}

#[test]
fn sqlite_history_deadline_returns_a_partial_report() {
    let (dir, engine, principal, scope) = setup();
    for index in 0..10_000 {
        std::fs::write(dir.path().join(format!("root/item-{index}")), [0]).unwrap();
    }
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "deadline-fixture").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let report = engine
        .compare_revisions_bounded(
            &revision,
            &revision,
            0,
            diskgraph_core::QueryBudget {
                deadline_ms: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(report.to_json(None)["complete"], false);
    assert_eq!(report.to_json(None)["truncation_reason"], "deadline");
}

#[test]
fn revoking_a_running_job_prevents_publication() {
    let (dir, engine, principal, scope) = setup();
    for index in 0..5000 {
        std::fs::write(dir.path().join(format!("root/revoke-{index}")), [0]).unwrap();
    }
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let engine = std::sync::Arc::new(engine);
    let worker = {
        let engine = engine.clone();
        let id = job.job_id.clone();
        std::thread::spawn(move || engine.run_job(&id, "revocation-worker"))
    };
    let started = std::time::Instant::now();
    loop {
        let state = engine
            .control_store()
            .unwrap()
            .job(&job.job_id)
            .unwrap()
            .state;
        if state == diskgraph_store::JobState::Running {
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        std::thread::yield_now();
    }
    engine
        .revoke_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    assert!(worker.join().unwrap().is_err());
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}

#[test]
fn matching_owner_name_cannot_steal_an_unexpired_running_job() {
    let (_dir, engine, principal, scope) = setup();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine
        .control_store()
        .unwrap()
        .claim_job(&job.job_id, "same-owner")
        .unwrap();
    assert!(engine.run_job(&job.job_id, "same-owner").is_err());
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}
