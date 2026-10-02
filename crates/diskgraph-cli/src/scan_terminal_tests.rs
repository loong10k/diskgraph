//! 已由其他 owner 完成的失败任务不能被 CLI --wait 报告为成功。
use diskgraph_core::{BusinessError, PrincipalId, ScanBudget};
use diskgraph_engine::{Engine, EngineConfig, EngineError};

#[test]
fn waiting_for_another_owners_failed_job_is_not_success() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), "data").unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        max_nodes_per_scan: 1,
        scan_budget: ScanBudget {
            max_nodes: 1,
            ..ScanBudget::default()
        },
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("fixture").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    assert!(matches!(
        engine.run_job(&job.job_id, "other-owner"),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert!(matches!(
        super::wait_for_terminal(&engine, &job.job_id, "waiter"),
        Err(EngineError::Business(BusinessError::Partial))
    ));
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}

#[test]
fn synchronous_wait_reclaims_only_its_expired_job() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), "data").unwrap();
    let data = directory.path().join("data");
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("fixture").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine
        .control_store()
        .unwrap()
        .claim_job_once(&job.job_id, "dead-owner")
        .unwrap();
    let observer = rusqlite::Connection::open(data.join("diskgraph-control.sqlite")).unwrap();
    observer
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0,heartbeat_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    let another_root = directory.path().join("another_project");
    std::fs::create_dir(&another_root).unwrap();
    let another_scope = engine
        .register_scope(
            &another_root,
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let untouched = engine
        .index_scope(
            &another_scope,
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let finished = super::wait_for_terminal_until(
        &engine,
        &job.job_id,
        "cli-recovery",
        std::time::Instant::now() + std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(finished.state, diskgraph_store::JobState::Completed);
    assert_eq!(finished.fencing_token, 2);
    assert_ne!(finished.owner, "dead-owner");
    let pending = engine.job_status(&untouched.job_id).unwrap();
    assert_eq!(pending.state, diskgraph_store::JobState::Queued);
    assert_eq!(pending.fencing_token, 0);
}

#[test]
fn synchronous_wait_settles_only_its_expired_cancelled_or_revoked_job() {
    for revoke in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let data = directory.path().join("data");
        let engine = Engine::open(EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("fixture").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let job = engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        engine
            .control_store()
            .unwrap()
            .claim_job_once(&job.job_id, "dead-owner")
            .unwrap();
        let observer = rusqlite::Connection::open(data.join("diskgraph-control.sqlite")).unwrap();
        if revoke {
            observer
                .execute(
                    "UPDATE scopes SET revoked=1 WHERE scope_id=?1",
                    [scope.as_str()],
                )
                .unwrap();
        } else {
            observer
                .execute(
                    "UPDATE jobs SET cancel_requested=1 WHERE job_id=?1",
                    [&job.job_id],
                )
                .unwrap();
        }
        // 真实存活租约不得被取消回收抢占。
        assert!(matches!(
            super::wait_for_terminal_until(
                &engine,
                &job.job_id,
                "waiter",
                std::time::Instant::now() + std::time::Duration::from_millis(100)
            ),
            Err(EngineError::Business(BusinessError::Timeout))
        ));
        assert_eq!(
            engine.job_status(&job.job_id).unwrap().state,
            diskgraph_store::JobState::Running
        );
        observer
            .execute(
                "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
                [&job.job_id],
            )
            .unwrap();
        // 同样过期、不可认领的其他任务不能由这个等待者顺带修改。
        let another_root = directory.path().join("another_project");
        std::fs::create_dir(&another_root).unwrap();
        let another_scope = engine
            .register_scope(
                &another_root,
                &principal,
                &engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        let another_job = engine
            .index_scope(
                &another_scope,
                &principal,
                &engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        engine
            .control_store()
            .unwrap()
            .claim_job_once(&another_job.job_id, "another-dead-owner")
            .unwrap();
        observer
            .execute(
                "UPDATE jobs SET lease_expires_unix_ms=0,cancel_requested=1 WHERE job_id=?1",
                [&another_job.job_id],
            )
            .unwrap();
        assert!(matches!(
            super::wait_for_terminal_until(
                &engine,
                &job.job_id,
                "waiter",
                std::time::Instant::now() + std::time::Duration::from_secs(2)
            ),
            Err(EngineError::Business(BusinessError::Partial))
        ));
        let finished = engine.job_status(&job.job_id).unwrap();
        assert_eq!(finished.state, diskgraph_store::JobState::Cancelled);
        assert_eq!(finished.fencing_token, 1);
        assert_eq!(
            engine.job_status(&another_job.job_id).unwrap().state,
            diskgraph_store::JobState::Running
        );
        assert!(engine.latest_revision(&scope).unwrap().is_none());
    }
}
