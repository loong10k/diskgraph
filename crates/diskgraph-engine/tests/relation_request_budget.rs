//! D23 真实请求预算回归，数据库与扫描根均为独占临时夹具。

// Linux 的扫描回归显式持有真实宿主；其他平台保留各自既有构造路径。
#[cfg(target_os = "linux")]
#[path = "support/native_scan_engine.rs"]
mod native_scan_engine;
#[cfg(not(target_os = "linux"))]
use diskgraph_engine::Engine;
#[cfg(target_os = "linux")]
use native_scan_engine::NativeScanEngine as Engine;

use diskgraph_core::{
    Authorizer, Decision, Permission, PolicyAuthorizer, PrincipalId, QueryBudget, ScopeId,
    TruncationReason,
};
use diskgraph_engine::EngineConfig;
use rusqlite::{Connection, params};
use std::time::{Duration, Instant};

struct Fixture {
    _directory: tempfile::TempDir,
    engine: Engine,
    principal: PrincipalId,
    revision: String,
    snapshot: String,
    db: Connection,
    root_id: i64,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload"), [0; 4096]).unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("request-budget").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let policy = engine.policy_authorizer().unwrap();
        let scope = engine.register_scope(&root, &principal, &policy).unwrap();
        let job = engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        engine.run_job(&job.job_id, "budget-owner").unwrap();
        let revision = engine.latest_revision(&scope).unwrap().unwrap();
        let snapshot = engine
            .revision_reader()
            .unwrap()
            .revision(&revision)
            .unwrap()
            .snapshot_id;
        let mut store = diskgraph_store::SqliteSnapshotStore::open(
            &directory.path().join("data/diskgraph.sqlite"),
        )
        .unwrap();
        let run = diskgraph_core::CollectorRun {
            run_id: "request-budget-relations".into(),
            snapshot_id: snapshot.clone(),
            collector_id: "request-budget-fixture".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: 1,
            coverage_complete: true,
            errors: vec![],
        };
        let owner = store.revision_ownership(&revision).unwrap().unwrap();
        let next_revision = format!("{revision}-fixture");
        let batch = diskgraph_core::CollectorBatch {
            run: run.clone(),
            entities: vec![],
            evidence: vec![],
            edges: vec![],
        };
        store
            .publish_collector_revision(
                &revision,
                &next_revision,
                2,
                (&owner.0, &owner.1),
                &batch,
                &[(&run.run_id, "active")],
            )
            .unwrap();
        let revision = next_revision;
        drop(store);
        let db = Connection::open(directory.path().join("data/diskgraph.sqlite")).unwrap();
        let root_id = db
            .query_row(
                "SELECT id FROM nodes WHERE snapshot_id=?1 AND parent_id IS NULL",
                [&snapshot],
                |row| row.get(0),
            )
            .unwrap();
        Self {
            _directory: directory,
            engine,
            principal,
            revision,
            snapshot,
            db,
            root_id,
        }
    }

    // 原始 JSON/BLOB/高扇出测试仍直接注入载荷，只补充合法 revision 选择。
    fn bind_edge(&self, edge_id: &str) {
        self.db
            .execute(
                "INSERT INTO relation_run_memberships VALUES (?1,'request-budget-relations',?2)",
                params![self.snapshot, edge_id],
            )
            .unwrap();
    }

    fn evidence(&self, subject: &str, confidence: u64) {
        let edge = serde_json::json!({"node_id":self.root_id,"relation":"rebuildable","subject":subject,"source":"test","observed_at_unix_ms":1,"confidence":confidence});
        self.db
            .execute(
                "INSERT INTO evidence(snapshot_id,node_id,evidence_json) VALUES (?1,?2,?3)",
                params![self.snapshot, self.root_id, edge.to_string()],
            )
            .unwrap();
    }
}

/// 在真实归属已经准入后的授权阶段耗尽同一绝对期限，不猜测宿主准备时长。
struct ExpireDuringAuthorization {
    policy: PolicyAuthorizer,
    deadline: Instant,
}
impl Authorizer for ExpireDuringAuthorization {
    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        std::thread::sleep(
            self.deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
        );
        self.policy.decide(principal, permission, scope)
    }
    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }
}

#[test]
fn authorization_time_exhausts_empty_and_positive_candidate_requests() {
    let f = Fixture::new();
    f.evidence("one", 100);
    let observed: Vec<_> = [0, 1]
        .into_iter()
        .map(|target| {
            let budget = QueryBudget {
                deadline_ms: 1000,
                ..QueryBudget::default()
            };
            let deadline = diskgraph_core::query_deadline(budget).unwrap();
            let auth = ExpireDuringAuthorization {
                policy: f.engine.policy_authorizer().unwrap(),
                deadline,
            };
            (
                target,
                f.engine
                    .review_candidates_until(
                        &f.revision,
                        target,
                        budget,
                        &f.principal,
                        &auth,
                        deadline,
                    )
                    .unwrap(),
            )
        })
        .collect();
    for (target, result) in observed {
        assert!(!result.complete, "late target {target} was complete");
        assert_eq!(result.truncated, Some(TruncationReason::Deadline));
        assert_eq!(result.remaining_bytes, target);
        assert!(result.candidates.is_empty());
    }
}

#[test]
fn authorization_time_exhausts_empty_impact_request() {
    let f = Fixture::new();
    let budget = QueryBudget {
        deadline_ms: 1000,
        ..QueryBudget::default()
    };
    let deadline = diskgraph_core::query_deadline(budget).unwrap();
    let auth = ExpireDuringAuthorization {
        policy: f.engine.policy_authorizer().unwrap(),
        deadline,
    };
    let result = f
        .engine
        .revision_impact_until(&f.revision, "absent", budget, &f.principal, &auth, deadline)
        .unwrap();
    assert_eq!(result.truncated, Some(TruncationReason::Deadline));
    assert!(result.entries.is_empty());
}

#[test]
fn expired_owner_preparation_refuses_a_fabricated_authorized_prefix() {
    let f = Fixture::new();
    let policy = f.engine.policy_authorizer().unwrap();
    let deadline = Instant::now() - Duration::from_millis(1);
    for result in [
        f.engine
            .review_candidates_until(
                &f.revision,
                1,
                QueryBudget::default(),
                &f.principal,
                &policy,
                deadline,
            )
            .map(|_| ()),
        f.engine
            .revision_impact_until(
                &f.revision,
                "absent",
                QueryBudget::default(),
                &f.principal,
                &policy,
                deadline,
            )
            .map(|_| ()),
    ] {
        assert!(matches!(
            result,
            Err(diskgraph_engine::EngineError::Business(
                diskgraph_core::BusinessError::BudgetExceeded
            )) | Err(diskgraph_engine::EngineError::Store(
                diskgraph_store::StoreError::BudgetExceeded
            ))
        ));
    }
}

#[test]
fn candidate_required_evidence_is_atomic_under_the_edge_cap() {
    let f = Fixture::new();
    f.evidence("one", 100);
    f.evidence("two", 100);
    let result = f
        .engine
        .review_candidates(
            &f.revision,
            1,
            QueryBudget {
                max_edges: 1,
                ..QueryBudget::default()
            },
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert_eq!(result.truncated, Some(TruncationReason::EdgeLimit));
    assert!(!result.complete);
    assert!(result.candidates.is_empty());
    assert_eq!((result.selected_bytes, result.remaining_bytes), (0, 1));
}

#[test]
fn oversized_candidate_raw_evidence_is_refused_before_invalid_u8_decoding() {
    let f = Fixture::new();
    f.evidence(&"x".repeat(100_000), 300);
    let result = f
        .engine
        .review_candidates(
            &f.revision,
            1,
            QueryBudget::default(),
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert_eq!(result.truncated, Some(TruncationReason::ByteLimit));
    assert!(result.candidates.is_empty());
    assert_eq!((result.selected_bytes, result.remaining_bytes), (0, 1));
}

#[test]
fn escaped_impact_data_must_fit_the_actual_json_budget() {
    let f = Fixture::new();
    let target = "\0".repeat(11_000);
    let edge = serde_json::json!({"edge_id":"edge", "source_entity_id":"start", "relation":"rebuildable_by", "target_entity_id":target, "assertion_kind":"observed", "evidence_refs":[]});
    f.db.execute("INSERT INTO relations(snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) VALUES (?1,?2,?3,?4,?5,?6)", params![f.snapshot,"edge","start",target,"rebuildable_by",edge.to_string()]).unwrap();
    f.bind_edge("edge");
    let result = f
        .engine
        .revision_impact(
            &f.revision,
            "start",
            QueryBudget::default(),
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let data = serde_json::json!({"entries":result.entries.iter().map(|entry|serde_json::json!({"entity_id":entry.entity_id,"relation":entry.relation.wire_name(),"depth":entry.depth})).collect::<Vec<_>>(),"complete":result.truncated.is_none(),"truncated":result.truncated.map(|reason|reason.wire_name()),"grants_execution":false});
    assert!(data.to_string().len() <= QueryBudget::default().max_response_bytes);
    assert_eq!(result.truncated, Some(TruncationReason::ByteLimit));
}

#[test]
fn decoded_nonpropagating_edges_consume_the_same_budget_as_outgoing_edges() {
    let f = Fixture::new();
    for (id, source, relation, target) in [
        ("a", "other", "protected_by", "start"),
        ("b", "start", "rebuildable_by", "recipe"),
    ] {
        let edge = serde_json::json!({"edge_id":id, "source_entity_id":source, "relation":relation, "target_entity_id":target, "assertion_kind":"observed", "evidence_refs":[]});
        f.db.execute("INSERT INTO relations(snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) VALUES (?1,?2,?3,?4,?5,?6)", params![f.snapshot,id,source,target,relation,edge.to_string()]).unwrap();
        f.bind_edge(id);
    }
    let result = f
        .engine
        .revision_impact(
            &f.revision,
            "start",
            QueryBudget {
                max_edges: 1,
                ..QueryBudget::default()
            },
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert!(
        result.entries.is_empty(),
        "a nonpropagating incoming edge must still consume its decode allowance"
    );
    assert_eq!(result.truncated, Some(TruncationReason::EdgeLimit));
}

#[test]
fn two_direction_pages_cannot_exceed_a_single_output_node_slot() {
    let f = Fixture::new();
    for (id, source, relation, target) in [
        ("incoming", "a", "contains", "start"),
        ("outgoing", "start", "rebuildable_by", "b"),
    ] {
        let edge = serde_json::json!({"edge_id":id,"source_entity_id":source,"target_entity_id":target,"relation":relation,"assertion_kind":"observed","evidence_refs":[]});
        f.db.execute(
            "INSERT INTO relations(snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) VALUES (?1,?2,?3,?4,?5,?6)",
            params![f.snapshot,id,source,target,relation,edge.to_string()],
        ).unwrap();
        f.bind_edge(id);
    }
    let result = f
        .engine
        .revision_impact(
            &f.revision,
            "start",
            QueryBudget {
                max_nodes: 1,
                max_edges: 2,
                ..QueryBudget::default()
            },
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert_eq!(
        result.entries.len(),
        1,
        "two already-decoded direction pages exceeded the output node cap"
    );
    assert_eq!(result.entries[0].entity_id, "a");
    assert_eq!(result.truncated, Some(TruncationReason::EdgeLimit));
}

#[test]
fn impact_keeps_its_text_contract_while_filtered_relations_keep_blob_compatibility() {
    let f = Fixture::new();
    let edge = serde_json::json!({"edge_id":"blob-edge","source_entity_id":"blob-start","target_entity_id":"blob-target","relation":"rebuildable_by","assertion_kind":"observed","evidence_refs":[]});
    let bytes = serde_json::to_vec(&edge).unwrap();
    assert!(
        bytes.len() < 4096,
        "this is an in-budget column type regression"
    );
    f.db.execute(
        "INSERT INTO relations(snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) VALUES (?1,'blob-edge','blob-start','blob-target','rebuildable_by',?2)",
        params![f.snapshot,bytes],
    ).unwrap();
    f.bind_edge("blob-edge");
    let policy = f.engine.policy_authorizer().unwrap();
    let impact = f.engine.revision_impact(
        &f.revision,
        "blob-start",
        QueryBudget::default(),
        &f.principal,
        &policy,
    );
    let filtered = f
        .engine
        .related_bounded(
            &f.revision,
            "blob-start",
            None,
            true,
            None,
            1,
            &f.principal,
            &policy,
        )
        .unwrap();
    assert_eq!(
        filtered["edges"].as_array().unwrap().len(),
        1,
        "the old filtered byte-reader path still accepts a JSON BLOB"
    );
    assert!(
        matches!(
            impact,
            Err(diskgraph_engine::EngineError::Store(
                diskgraph_store::StoreError::Sqlite(_)
            ))
        ),
        "impact silently promoted a BLOB column that its old TEXT page rejects"
    );
}
