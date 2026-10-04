//! 已提交后回收结果的历史状态；来源：公开 Engine 扫描、Git 发布及 prune 生命周期。
use crate::git_evidence_fixture::{GitEvidenceFixture, now};
use diskgraph_core::Authorizer;
use diskgraph_store::JobState;

#[test]
fn pruned_git_result_keeps_authorized_receipt_and_marks_result_unavailable() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    assert_eq!(
        f.engine
            .run_job_strict(&job.job_id, "status-publisher")
            .unwrap()
            .state,
        JobState::Completed
    );
    let receipt = f
        .engine
        .revision_reader()
        .unwrap()
        .job_publication_receipt(&job.job_id)
        .unwrap()
        .unwrap();
    let policy = f.engine.policy_authorizer().unwrap();
    let before = f
        .engine
        .git_job_status_details(&job.job_id, &f.actor, &policy)
        .unwrap()
        .unwrap();
    // 先观察真实已存在结果；再生成新快照替换 latest，使旧 Git revision 合法可回收。
    let scan = f.engine.index_scope(&f.scope, &f.actor, &policy).unwrap();
    assert_eq!(
        f.engine
            .run_job(&scan.job_id, "replacement-scan")
            .unwrap()
            .state,
        JobState::Completed
    );
    let removed = f
        .engine
        .prune_snapshots(&f.scope, 1, true, &f.actor, &policy)
        .unwrap();
    assert!(
        removed
            .iter()
            .any(|revision| revision.revision_id == receipt.revision_id())
    );
    assert!(
        f.engine
            .revision_reader()
            .unwrap()
            .revision(receipt.revision_id())
            .is_err()
    );
    let after = f
        .engine
        .git_job_status_details(&job.job_id, &f.actor, &policy)
        .unwrap()
        .unwrap();
    assert_eq!(after["state"], "completed");
    assert_eq!(after["revision_id"], receipt.revision_id());
    assert_eq!(after["revision"], receipt.revision_id());
    assert_eq!(after["run_id"], receipt.run_id());
    assert_eq!(
        after["result_available"], false,
        "tombstone is committed history, not available evidence"
    );
    assert_eq!(before["result_available"], true);
    assert_eq!(
        f.engine
            .revision_reader()
            .unwrap()
            .job_publication_receipt(&job.job_id)
            .unwrap()
            .unwrap(),
        receipt
    );
}

#[test]
fn completed_git_status_requires_live_metadata_permission_in_addition_to_operation_view() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    f.engine
        .run_job_strict(&job.job_id, "status-publisher")
        .unwrap();
    let original_ceiling = f.engine.policy_authorizer().unwrap();
    f.engine
        .control()
        .unwrap()
        .revoke_grant(
            &f.actor,
            &diskgraph_core::Permission::MetadataRead,
            &f.scope,
        )
        .unwrap();
    let result = f
        .engine
        .git_job_status_details(&job.job_id, &f.actor, &original_ceiling);
    assert!(matches!(
        result,
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::PermissionDenied
        ))
    ));
}

/// 在完成状态的最后一次能力回调撤销前一项 grant；来源：独立 SQLite 控制连接。
struct TerminalRevocation {
    policy: diskgraph_core::PolicyAuthorizer,
    database: std::path::PathBuf,
    calls: std::cell::Cell<usize>,
}
impl Authorizer for TerminalRevocation {
    fn decide(
        &self,
        principal: &diskgraph_core::PrincipalId,
        permission: &diskgraph_core::Permission,
        scope: &diskgraph_core::ScopeId,
    ) -> diskgraph_core::Decision {
        let calls = self.calls.get() + 1;
        self.calls.set(calls);
        if calls == 4 {
            assert_eq!(*permission, diskgraph_core::Permission::MetadataRead);
            let changed = rusqlite::Connection::open(&self.database)
                .unwrap()
                .execute(
                    "DELETE FROM grants WHERE principal_id=?1 AND scope_id=?2 AND permission=?3",
                    rusqlite::params![
                        principal.as_str(),
                        scope.as_str(),
                        diskgraph_core::Permission::OperationView.wire_name()
                    ],
                )
                .unwrap();
            assert_eq!(changed, 1);
        }
        self.policy.decide(principal, permission, scope)
    }
}

#[test]
fn completed_git_status_rechecks_all_live_grants_after_the_last_terminal_callback() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    f.engine
        .run_job_strict(&job.job_id, "status-publisher")
        .unwrap();
    let authorization = TerminalRevocation {
        policy: f.engine.policy_authorizer().unwrap(),
        database: f.temp.path().join("data/diskgraph-control.sqlite"),
        calls: std::cell::Cell::new(0),
    };
    let result = f
        .engine
        .git_job_status_details(&job.job_id, &f.actor, &authorization);
    assert_eq!(
        authorization.calls.get(),
        4,
        "must reach final MetadataRead callback after preparing actual receipt response"
    );
    assert!(matches!(
        result,
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::PermissionDenied
        ))
    ));
}
