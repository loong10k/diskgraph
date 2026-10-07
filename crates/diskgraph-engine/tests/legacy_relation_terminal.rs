//! 旧公共关系签名的结果兼容与实际末段撤权；隔离导入不声称原生扫描。
#![allow(deprecated)] // 明确验证已弃用的旧签名，新生产请求使用完整预算入口。
use diskgraph_core::{Authorizer, Decision, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
use diskgraph_engine::EngineError;
use diskgraph_store::ControlStore;
use std::cell::Cell;

#[path = "relation_preparation_budget/fixture.rs"]
mod fixture;
use fixture::Fixture;

#[test]
fn old_relation_results_preserve_entity_edges_evidence_and_absent_semantics() {
    let empty = Fixture::new(8);
    assert!(
        empty
            .base
            .engine
            .explain_entity(
                "selected",
                "absent",
                &empty.base.principal,
                &empty.base.policy
            )
            .unwrap()
            .is_none()
    );
    let fixture = Fixture::with_edge(8, 16);
    let engine = &fixture.base.engine;
    let principal = &fixture.base.principal;
    let policy = &fixture.base.policy;
    let explanation = engine
        .explain_entity("selected", "entity", principal, policy)
        .unwrap()
        .unwrap();
    assert_eq!(explanation.0.entity_id, "entity");
    assert_eq!(explanation.1.len(), 1);
    assert_eq!(explanation.2.len(), 1);
    assert_eq!(explanation.2[0].evidence_id, "fixture-evidence");
    let edges = engine
        .related("selected", "entity", None, false, principal, policy)
        .unwrap();
    assert_eq!(edges, explanation.1);
    let (page, more) = engine
        .related_page("selected", "entity", false, None, 1, principal, policy)
        .unwrap();
    assert_eq!(page, edges);
    assert!(!more);
    let (next, more) = engine
        .related_page(
            "selected",
            "entity",
            false,
            Some("edge"),
            1,
            principal,
            policy,
        )
        .unwrap();
    assert!(next.is_empty());
    assert!(!more);
    assert!(
        fixture
            .data_dir()
            .join("diskgraph-control.sqlite")
            .is_file()
    );
}

#[test]
fn old_relation_signatures_recheck_real_grant_after_reading() {
    /// 只在能力回调中撤销真实 grant，不伪造读取结果或持久授权决定。
    struct RevokeDuringDecision<'a> {
        data_dir: std::path::PathBuf,
        policy: &'a PolicyAuthorizer,
        calls: Cell<usize>,
    }
    impl Authorizer for RevokeDuringDecision<'_> {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if call == 2 {
                ControlStore::open(&self.data_dir.join("diskgraph-control.sqlite"))
                    .unwrap()
                    .revoke_grant(principal, permission, scope)
                    .unwrap();
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    for mode in 0..3 {
        let fixture = Fixture::with_edge(8, 16);
        let auth = RevokeDuringDecision {
            data_dir: fixture.data_dir(),
            policy: &fixture.base.policy,
            calls: Cell::new(0),
        };
        let engine = &fixture.base.engine;
        let principal = &fixture.base.principal;
        let result = match mode {
            0 => engine
                .explain_entity("selected", "entity", principal, &auth)
                .map(|_| ()),
            1 => engine
                .related("selected", "entity", None, false, principal, &auth)
                .map(|_| ()),
            2 => engine
                .related_page("selected", "entity", false, None, 1, principal, &auth)
                .map(|_| ()),
            _ => unreachable!(),
        };
        assert!(
            matches!(
                result,
                Err(EngineError::Business(
                    diskgraph_core::BusinessError::PermissionDenied
                ))
            ),
            "mode {mode}: {result:?}"
        );
        assert!(auth.calls.get() >= 2);
        assert!(
            !ControlStore::open(&fixture.data_dir().join("diskgraph-control.sqlite"))
                .unwrap()
                .all_grants()
                .unwrap()
                .iter()
                .any(|grant| grant.principal == *principal
                    && grant.permission == Permission::MetadataRead
                    && grant.scope == fixture.scope)
        );
    }
}
