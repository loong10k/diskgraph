//! 没有受信 helper 宿主时真实 Index 执行必须拒绝；来源：PF-06。
//! 使用现有公开 Engine/Store 入口，不依赖计划中新方法或伪造 Child。

use diskgraph_core::{
    Authorizer, BusinessError, Decision, Permission, PolicyAuthorizer, PrincipalId,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError, admin_scope};
use diskgraph_store::JobState;

#[test]
fn absent_trusted_host_refuses_actual_index_without_publication() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("registered-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("one"), b"observed file").unwrap();
    let config = EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    };
    let engine = Engine::open(config.clone()).unwrap();
    let actor = PrincipalId::new("trusted-host-test").unwrap();
    let mut policy = PolicyAuthorizer::new(1);
    policy.grant(actor.clone(), Permission::ScopeAdmin, admin_scope());
    engine.bootstrap_local_admin(&actor).unwrap();
    assert_eq!(
        engine
            .policy_authorizer()
            .unwrap()
            .decide(&actor, &Permission::ScopeAdmin, &admin_scope()),
        Decision::Allowed,
        "public bootstrap must establish the actual administrative grant"
    );
    let scope = engine.register_scope(&root, &actor, &policy).unwrap();
    for permission in [
        Permission::IndexWrite,
        Permission::MetadataRead,
        Permission::OperationView,
    ] {
        policy.grant(actor.clone(), permission, scope.clone());
    }
    assert_eq!(
        engine
            .control_store()
            .unwrap()
            .live_permission(&actor, &Permission::IndexWrite, &scope)
            .unwrap(),
        Some(true)
    );
    let job = engine.index_scope(&scope, &actor, &policy).unwrap();
    assert_eq!(job.state, JobState::Queued);
    let result = engine.run_job(&job.job_id, "real-no-host-owner");
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::Unsupported))
        ),
        "actual no-host execution: {result:?}"
    );
    let terminal = engine.job_status(&job.job_id).unwrap();
    assert_eq!(terminal.state, JobState::Failed);
    assert_eq!(terminal.owner, "real-no-host-owner");
    assert!(terminal.fencing_token > 0);
    assert!(
        !engine
            .control_store()
            .unwrap()
            .cancellation_requested(&job.job_id, terminal.fencing_token)
            .unwrap()
    );
    assert_eq!(engine.latest_revision(&scope).unwrap(), None);
    assert_eq!(engine.scope(&scope).unwrap().scope_id, scope);
    // 旧 open 仍能重新打开查询/管理两库；拒绝扫描不撤销既有 scope/grant。
    drop(engine);
    let reopened = Engine::open(config).unwrap();
    assert_eq!(reopened.job_status(&job.job_id).unwrap(), terminal);
    assert_eq!(reopened.latest_revision(&scope).unwrap(), None);
    assert_eq!(
        reopened
            .control_store()
            .unwrap()
            .live_permission(&actor, &Permission::IndexWrite, &scope)
            .unwrap(),
        Some(true)
    );
}
