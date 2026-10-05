//! 错误传播测试的真实数据库资格与终态检查；来源：原生 Rust Engine / SQLite。

use crate::EngineError;
use crate::git_evidence_fixture::GitEvidenceFixture;
use diskgraph_core::Permission;
use diskgraph_store::{JobRecord, JobState, StoreError};
use std::time::{SystemTime, UNIX_EPOCH};

/// 参数：fixture 为独占真实仓库；返回原公开 Index 入队记录，不改扫描预算。
pub(super) fn enqueue_scan(f: &GitEvidenceFixture) -> JobRecord {
    f.engine
        .index_scope(&f.scope, &f.actor, &f.engine.policy_authorizer().unwrap())
        .unwrap()
}

/// 参数：fixture/claimed/owner 为当前实际执行代次；返回无，确认原租约及真实 fence 有效。
pub(super) fn qualify_claim(f: &GitEvidenceFixture, claimed: &JobRecord, owner: &str) {
    assert_eq!(claimed.state, JobState::Running);
    assert_eq!(claimed.owner, owner);
    assert!(claimed.fencing_token > 0);
    assert!(lease_live(claimed));
    let mut control = f.engine.control_store().unwrap();
    assert_eq!(control.job(&claimed.job_id).unwrap(), *claimed);
    control
        .with_job_fence(&claimed.job_id, owner, claimed.fencing_token, || Ok(()))
        .unwrap();
}

/// 参数：claimed 为真实认领记录；返回当前墙钟仍在原租约内，不续租或调整测试时钟。
pub(super) fn lease_live(claimed: &JobRecord) -> bool {
    u128::from(claimed.lease_expires_unix_ms)
        > SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
}

/// 参数：fixture/permission 为实际授予的权限；返回无，独立连接精确撤掉一条真实 grant。
pub(super) fn withdraw(f: &GitEvidenceFixture, permission: Permission) {
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .live_permission(&f.actor, &permission, &f.scope)
            .unwrap(),
        Some(true)
    );
    let connection =
        rusqlite::Connection::open(f.engine.data_dir().join("diskgraph-control.sqlite")).unwrap();
    let changed = connection
        .execute(
            "DELETE FROM grants WHERE principal_id=?1 AND permission=?2 AND scope_id=?3",
            rusqlite::params![f.actor.as_str(), permission.wire_name(), f.scope.as_str()],
        )
        .unwrap();
    assert_eq!(changed, 1, "must withdraw exactly one actual grant");
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .live_permission(&f.actor, &permission, &f.scope)
            .unwrap(),
        Some(false)
    );
}

/// 参数：fixture/job/owner 为已结束执行；返回无，不将本机拒权或 SQL 错误伪写为用户取消。
pub(super) fn assert_failed_without_caller_cancel(f: &GitEvidenceFixture, job: &str, owner: &str) {
    let terminal = f.engine.job_status(job).unwrap();
    assert_eq!(terminal.state, JobState::Failed);
    assert_eq!(terminal.owner, owner);
    assert!(terminal.fencing_token > 0);
    assert!(
        !f.engine
            .control_store()
            .unwrap()
            .cancellation_requested(job, terminal.fencing_token)
            .unwrap()
    );
    assert!(!f.engine.cancellations().unwrap().contains_key(job));
    assert!(
        !f.engine
            .scan_progress
            .lock()
            .unwrap()
            .keys()
            .any(|(id, _)| id == job)
    );
    f.assert_no_git_publication();
}

/// 参数：error 为真实 keeper/heartbeat 返回；返回是否为普通 SQLite BUSY，排除 LOCKED/快照/中断。
pub(super) fn sqlite_busy(error: &EngineError) -> bool {
    matches!(error,
        EngineError::Store(StoreError::Sqlite(rusqlite::Error::SqliteFailure(code, _)))
        if code.code == rusqlite::ErrorCode::DatabaseBusy && code.extended_code == 5)
}
