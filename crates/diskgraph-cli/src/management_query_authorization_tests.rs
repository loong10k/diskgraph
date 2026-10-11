//! 持久排队任务和合成快照经真实CLI分发；不启动扫描或服务。
use crate::{cli::Cli, dispatch::dispatch, narrow_query_authorization_tests::fixture};
use clap::Parser;
use diskgraph_core::{BusinessError, PrincipalId};
use diskgraph_engine::EngineError;
use diskgraph_store::{ControlStore, JobKind};
use std::time::{Duration, Instant};

#[test]
fn actual_scan_job_status_refuses_ungranted_actor() {
    let (directory, engine, owner, scope) = fixture();
    let job = ControlStore::open(&directory.path().join("data/diskgraph-control.sqlite"))
        .unwrap()
        .create_job(&scope, JobKind::Index, &owner)
        .unwrap();
    let cli = Cli::parse_from(["diskgraph", "--json", "status", "--job", &job.job_id]);
    let actor = PrincipalId::new("ungranted").unwrap();
    let mut output = vec![];
    let result = dispatch(
        &engine,
        &cli,
        &actor,
        &engine.policy_authorizer().unwrap(),
        &mut output,
        Instant::now() + Duration::from_secs(5),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "status escaped: {result:?} {output:?}"
    );
    assert!(output.is_empty());
}

#[test]
fn actual_management_queries_refuse_expired_dispatch() {
    let (directory, engine, actor, scope) = fixture();
    let job = ControlStore::open(&directory.path().join("data/diskgraph-control.sqlite"))
        .unwrap()
        .create_job(&scope, JobKind::Index, &actor)
        .unwrap();
    for args in [
        vec!["diskgraph", "--json", "status", "--job", &job.job_id],
        vec![
            "diskgraph",
            "--json",
            "snapshots",
            "--scope",
            scope.as_str(),
        ],
    ] {
        let cli = Cli::parse_from(args);
        let mut output = vec![];
        let result = dispatch(
            &engine,
            &cli,
            &actor,
            &engine.policy_authorizer().unwrap(),
            &mut output,
            Instant::now(),
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::BudgetExceeded))
            ),
            "management expired: {result:?} {output:?}"
        );
        assert!(output.is_empty());
    }
}

#[test]
fn management_queries_recheck_after_actual_encoding() {
    for command in ["status", "snapshots"] {
        let (directory, engine, actor, scope) = fixture();
        let path = directory.path().join("data/diskgraph-control.sqlite");
        let job = ControlStore::open(&path)
            .unwrap()
            .create_job(&scope, JobKind::Index, &actor)
            .unwrap();
        let args = if command == "status" {
            vec!["diskgraph", "--json", "status", "--job", &job.job_id]
        } else {
            vec![
                "diskgraph",
                "--json",
                "snapshots",
                "--scope",
                scope.as_str(),
            ]
        };
        let cli = Cli::parse_from(args);
        let policy = engine.policy_authorizer().unwrap();
        let revoked = scope.clone();
        crate::query_terminal_tests::arm_before_reply(move || {
            ControlStore::open(&path)
                .unwrap()
                .revoke_scope(&revoked)
                .unwrap()
        });
        let mut output = vec![];
        let result = dispatch(
            &engine,
            &cli,
            &actor,
            &policy,
            &mut output,
            Instant::now() + Duration::from_secs(5),
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "{command} encoded data escaped: {result:?} {output:?}"
        );
        assert!(output.is_empty());
    }
}

#[test]
fn management_queries_preserve_normal_status_and_snapshot_fields() {
    let (directory, engine, actor, scope) = fixture();
    let job = ControlStore::open(&directory.path().join("data/diskgraph-control.sqlite"))
        .unwrap()
        .create_job(&scope, JobKind::Index, &actor)
        .unwrap();
    for command in ["status", "snapshots"] {
        let args = if command == "status" {
            vec!["diskgraph", "--json", "status", "--job", &job.job_id]
        } else {
            vec![
                "diskgraph",
                "--json",
                "snapshots",
                "--scope",
                scope.as_str(),
            ]
        };
        let mut output = vec![];
        dispatch(
            &engine,
            &Cli::parse_from(args),
            &actor,
            &engine.policy_authorizer().unwrap(),
            &mut output,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output[0]).unwrap();
        if command == "status" {
            assert_eq!(value["data"]["state"], "queued");
            assert_eq!(value["data"]["job_id"], job.job_id);
        } else {
            assert_eq!(value["data"]["snapshots"][0]["snapshot_id"], "fixture");
        }
    }
}

#[test]
fn management_queries_remember_withdrawal_restored_after_encoding() {
    use diskgraph_core::Authorizer;
    for command in ["status", "snapshots"] {
        let (directory, engine, actor, scope) = fixture();
        let path = directory.path().join("data/diskgraph-control.sqlite");
        let job = ControlStore::open(&path)
            .unwrap()
            .create_job(&scope, JobKind::Index, &actor)
            .unwrap();
        let args = if command == "status" {
            vec!["diskgraph", "--json", "status", "--job", &job.job_id]
        } else {
            vec![
                "diskgraph",
                "--json",
                "snapshots",
                "--scope",
                scope.as_str(),
            ]
        };
        let cli = Cli::parse_from(args);
        let policy = engine.policy_authorizer().unwrap();
        let version = policy.policy_version();
        let target = scope.clone();
        let original_actor = actor.clone();
        let permission = if command == "status" {
            diskgraph_core::Permission::OperationView
        } else {
            diskgraph_core::Permission::MetadataRead
        };
        crate::query_terminal_tests::arm_before_reply(move || {
            let mut control = ControlStore::open(&path).unwrap();
            control
                .revoke_grant(&original_actor, &permission, &target)
                .unwrap();
            control
                .upsert_grant(&diskgraph_core::Grant {
                    principal: original_actor,
                    permission,
                    scope: target,
                    policy_version: version,
                })
                .unwrap();
        });
        let mut output = vec![];
        let result = dispatch(
            &engine,
            &cli,
            &actor,
            &policy,
            &mut output,
            Instant::now() + Duration::from_secs(5),
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(
                    BusinessError::PermissionDenied | BusinessError::Conflict
                ))
            ),
            "{command} restored grant lost withdrawal: {result:?} {output:?}"
        );
        assert!(output.is_empty());
    }
}
