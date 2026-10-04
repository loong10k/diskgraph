use crate::{ControlStore, SqliteSnapshotStore};
use diskgraph_core::{
    AssertionKind, CollectorBatch, CollectorRun, Entity, EntityKind, EvidenceRecord,
    GitEvidenceJobInput, GitEvidenceLimits, GitEvidenceSummary, Grant, JobRequestAuthority,
    Locator, LocatorKind, Permission, Polarity, PrincipalId, Relation, RelationEdge,
};

pub(super) fn fixture() -> (
    ControlStore,
    SqliteSnapshotStore,
    GitEvidenceJobInput,
    JobRequestAuthority,
) {
    let mut control = ControlStore::open_in_memory().unwrap();
    let server = control.ensure_server().unwrap();
    let scope = control
        .register_scope(
            &Locator {
                kind: LocatorKind::NativePath,
                raw_b64: "L2ZpeHR1cmU=".into(),
                display: "/fixture".into(),
            },
            None,
        )
        .unwrap();
    let principal = PrincipalId::new("alice").unwrap();
    control.publish_policy_version(1).unwrap();
    for permission in [
        Permission::MetadataRead,
        Permission::IndexWrite,
        Permission::ContentRead,
    ] {
        control
            .upsert_grant(&Grant {
                principal: principal.clone(),
                permission,
                scope: scope.clone(),
                policy_version: 1,
            })
            .unwrap();
    }
    let authority = JobRequestAuthority::authenticated_remote(
        principal,
        "issuer",
        "http",
        vec![
            Permission::MetadataRead,
            Permission::IndexWrite,
            Permission::ContentRead,
        ],
        ControlStore::now_ms() / 1000 + 60,
    )
    .unwrap();
    let input = GitEvidenceJobInput::new(
        server,
        scope,
        "base".into(),
        2,
        GitEvidenceLimits::default(),
    )
    .unwrap();
    let mut graph = crate::tests::graph("snapshot", 100);
    graph.evidence.clear();
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    // 公开 owned 发布只能使用实际 staging，不能绕过已完成扫描的行数合同。
    store.append_staging_nodes("initial", &graph.nodes).unwrap();
    store
        .publish_revision_owned(
            "initial",
            &graph,
            "base",
            100,
            Some((input.server_id().as_str(), input.scope_id().as_str())),
        )
        .unwrap();
    (control, store, input, authority)
}
pub(super) fn batch(input: &GitEvidenceJobInput, run: &str) -> CollectorBatch {
    let at = ControlStore::now_ms();
    let resource = format!("resource-{run}");
    let project = format!("project-{run}");
    let evidence = format!("evidence-{run}");
    let summary = GitEvidenceSummary::new(2, 1, None, None, at).unwrap();
    CollectorBatch {
        run: CollectorRun {
            run_id: run.into(),
            snapshot_id: "snapshot".into(),
            collector_id: "git-local".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: at,
            coverage_complete: true,
            errors: vec![],
        },
        entities: vec![
            Entity {
                entity_id: resource.clone(),
                kind: EntityKind::Resource,
                identity: serde_json::json!({"node_id":input.node_id()}).to_string(),
                display: "indexed resource".into(),
                source_run_id: run.into(),
            },
            Entity {
                entity_id: project.clone(),
                kind: EntityKind::Project,
                identity: serde_json::json!({"node_id":input.node_id(),"collector":"git-local"})
                    .to_string(),
                display: "Git project".into(),
                source_run_id: run.into(),
            },
        ],
        evidence: vec![EvidenceRecord {
            evidence_id: evidence.clone(),
            run_id: run.into(),
            basis: serde_json::to_string(&summary).unwrap(),
            observed_at_unix_ms: at,
            expires_at_unix_ms: Some(at + 30000),
            confidence: 90,
            input_fingerprint: summary.observation_fingerprint(input),
        }],
        edges: vec![RelationEdge {
            edge_id: format!("edge-{run}"),
            source_entity_id: resource,
            relation: Relation::OwnedByProject,
            target_entity_id: project,
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![(evidence, Polarity::Supports)],
        }],
    }
}
