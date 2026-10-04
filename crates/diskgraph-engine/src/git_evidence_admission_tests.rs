//! Git 入队原始头部预算；来源：公开 Store 发布的合法历史，而非损坏 SQL 夹具。
use crate::EngineError;
use crate::git_evidence_fixture::GitEvidenceFixture;
use diskgraph_core::{BusinessError, JobRequestAuthority, QueryBudget, QueryReadBudget};
use diskgraph_store::StoreError;
use std::time::{Duration, Instant};

#[test]
fn git_enqueue_admits_legal_revision_header_before_owning_snapshot_identity() {
    let f = GitEvidenceFixture::new();
    let reader = f.engine.revision_reader().unwrap();
    let mut graph = reader.load_revision(&f.base).unwrap();
    let mut reads = QueryReadBudget::new(
        QueryBudget {
            max_nodes: 10000,
            max_response_bytes: 16 << 20,
            ..QueryBudget::default()
        },
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    let located: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| {
            let locator = reader
                .native_locator_bounded(&graph.snapshot.id, node.id, &mut reads)
                .unwrap()
                .unwrap();
            let observed = reader
                .windows_observation_bounded(&graph.snapshot.id, node.id, &mut reads)
                .unwrap()
                .unwrap();
            (locator, observed)
        })
        .collect();
    drop(reader);
    // 旧可信 Store 接口接受此合法大 ID；新请求必须把头部与节点放在同一 64KiB 账本。
    graph.snapshot.id = "s".repeat(2 << 20);
    graph.evidence.clear();
    let server = f.engine.server_id().unwrap();
    {
        let mut store = f.engine.graph().unwrap();
        store
            .append_staging_observed_iter(
                "large-header",
                graph
                    .nodes
                    .iter()
                    .zip(&located)
                    .map(|(node, (locator, observed))| {
                        (
                            node,
                            locator.locator.as_ref().unwrap(),
                            locator.self_modified,
                            observed.observation.as_ref(),
                            observed.gap,
                        )
                    }),
            )
            .unwrap();
        store
            .publish_revision_owned(
                "large-header",
                &graph,
                "large-header-revision",
                123,
                Some((server.as_str(), f.scope.as_str())),
            )
            .unwrap();
    }
    let authority = JobRequestAuthority::trusted_local(f.actor.clone(), "trusted-engine").unwrap();
    let result = f.engine.git_evidence_scope_with_authority(
        &f.scope,
        "large-header-revision",
        f.node,
        &authority,
        &f.engine.policy_authorizer().unwrap(),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
                | Err(EngineError::Store(StoreError::BudgetExceeded))
        ),
        "legal oversized revision header escaped shared admission: {result:?}"
    );
}
