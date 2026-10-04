//! D42 纯存储协议夹具：强身份值为显式测试数据，不冒充实际 native/scanner 证明。
use crate::{ControlStore, SqliteSnapshotStore};
use diskgraph_core::{
    AssertionKind, CollectorBatch, CollectorRun, Entity, EntityKind, EvidenceRecord,
    IndexedFileEpoch, JobRequestAuthority, Locator, LocatorEncoding, LocatorKind, NodeKind,
    Permission, Polarity, PrincipalId, ProcessEvidenceJobInput, ProcessEvidenceLimits,
    ProcessEvidenceSummary, ProcessObservationCode, ProcessObservationCoverage,
    ProcessObservationMethod, ProcessStartupIdentity, QualifiedLocator, Relation, RelationEdge,
    ResourceLocator, UnixFileObservation,
};
pub(super) fn epoch() -> IndexedFileEpoch {
    IndexedFileEpoch::LinuxHandle {
        device: 1,
        inode: 2,
        filesystem_domain_sha256: [3; 32],
        handle_type: 1,
        handle_bytes: vec![4; 12],
    }
}
pub(super) fn fixture() -> (
    ControlStore,
    SqliteSnapshotStore,
    ProcessEvidenceJobInput,
    JobRequestAuthority,
) {
    let mut control = ControlStore::open_in_memory().unwrap();
    let server = control.ensure_server().unwrap();
    let scope = control
        .register_scope(
            &Locator::from_native_path(std::path::Path::new("/tmp/diskgraph-test")),
            None,
        )
        .unwrap();
    let principal = PrincipalId::new("process-observer").unwrap();
    control.publish_policy_version(1).unwrap();
    for permission in [Permission::MetadataRead, Permission::IndexWrite] {
        control
            .upsert_grant(&diskgraph_core::Grant {
                principal: principal.clone(),
                permission,
                scope: scope.clone(),
                policy_version: 1,
            })
            .unwrap();
    }
    let input = ProcessEvidenceJobInput::new(
        server,
        scope,
        "process-base".into(),
        2,
        ProcessObservationMethod::LinuxProcfsV1,
        epoch(),
        ProcessEvidenceLimits::default(),
    )
    .unwrap();
    let mut graph = crate::tests::graph("process-snapshot", 100);
    graph.evidence.clear();
    graph.nodes[0].directories = 0;
    graph.nodes[1].kind = NodeKind::File;
    graph.nodes[1].directories = 0;
    let locators: Vec<_> = graph
        .nodes
        .iter()
        .map(|n| {
            let ResourceLocator::NativePath(path) = &n.locator else {
                unreachable!()
            };
            QualifiedLocator::from_parts(
                LocatorKind::NativePath,
                LocatorEncoding::UnixBytes,
                path.as_bytes().to_vec(),
                path.clone(),
            )
            .unwrap()
        })
        .collect();
    let observation =
        UnixFileObservation::new(epoch(), 0o100600, 100, (1, 2), (3, 4), (10, 11)).unwrap();
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store
        .append_staging_located_iter(
            "process-stage",
            graph
                .nodes
                .iter()
                .zip(&locators)
                .map(|(n, l)| (n, l, Some(1))),
        )
        .unwrap();
    store
        .append_staging_unix_observations_checked(
            "process-stage",
            std::iter::once((2, Some(&observation), None)),
            || Ok(()),
        )
        .unwrap();
    store
        .publish_revision_owned(
            "process-stage",
            &graph,
            "process-base",
            100,
            Some((input.server_id().as_str(), input.scope_id().as_str())),
        )
        .unwrap();
    let authority = JobRequestAuthority::authenticated_remote(
        principal,
        "issuer",
        "http",
        vec![Permission::MetadataRead, Permission::IndexWrite],
        ControlStore::now_ms() / 1000 + 60,
    )
    .unwrap();
    (control, store, input, authority)
}
pub(super) fn batch(
    input: &ProcessEvidenceJobInput,
    run: &str,
    positive: bool,
    partial: bool,
) -> CollectorBatch {
    let at = ControlStore::now_ms();
    let resource = format!("resource-{run}");
    let ev = format!("evidence-{run}");
    let process_id = format!("process-{run}");
    let startup = ProcessStartupIdentity::Linux {
        pid: 7,
        start_ticks: 10,
        visibility_domain_sha256: [3; 32],
    };
    let summary = ProcessEvidenceSummary::new(
        input.method(),
        (at, at),
        if partial {
            ProcessObservationCoverage::Partial
        } else {
            ProcessObservationCoverage::VisibleMethodDomainComplete
        },
        [3; 32],
        if positive {
            vec![startup.clone()]
        } else {
            vec![]
        },
        if partial {
            vec![ProcessObservationCode::VisibilityRestricted]
        } else {
            vec![]
        },
    )
    .unwrap();
    let mut entities = vec![Entity {
        entity_id: resource.clone(),
        kind: EntityKind::Resource,
        identity: serde_json::json!({"node_id":input.node_id()}).to_string(),
        display: "indexed resource".into(),
        source_run_id: run.into(),
    }];
    if positive {
        entities.push(Entity {
            entity_id: process_id.clone(),
            kind: EntityKind::Process,
            identity: serde_json::json!({"server_id":input.server_id(),"startup":startup})
                .to_string(),
            display: "observed process".into(),
            source_run_id: run.into(),
        });
    }
    CollectorBatch {
        run: CollectorRun {
            run_id: run.into(),
            snapshot_id: "process-snapshot".into(),
            collector_id: "process-native".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: at,
            coverage_complete: !partial,
            errors: summary.codes().iter().map(|c| c.as_str().into()).collect(),
        },
        entities,
        evidence: vec![EvidenceRecord {
            evidence_id: ev.clone(),
            run_id: run.into(),
            basis: serde_json::to_string(&summary).unwrap(),
            observed_at_unix_ms: at,
            expires_at_unix_ms: Some(at + 30000),
            confidence: 90,
            input_fingerprint: summary.observation_fingerprint(input),
        }],
        edges: if positive {
            vec![RelationEdge {
                edge_id: format!("edge-{run}"),
                source_entity_id: resource,
                relation: Relation::UsedByProcess,
                target_entity_id: process_id,
                assertion_kind: AssertionKind::Observed,
                evidence_refs: vec![(ev, Polarity::Supports)],
            }]
        } else {
            vec![]
        },
    }
}
pub(super) fn next(input: &ProcessEvidenceJobInput, base: &str) -> ProcessEvidenceJobInput {
    ProcessEvidenceJobInput::new(
        input.server_id().clone(),
        input.scope_id().clone(),
        base.into(),
        input.node_id(),
        input.method(),
        input.indexed_epoch().clone(),
        input.limits().clone(),
    )
    .unwrap()
}
pub(super) fn count(store: &SqliteSnapshotStore, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
