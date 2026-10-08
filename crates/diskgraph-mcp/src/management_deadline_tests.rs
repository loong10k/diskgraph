//! 管理只读入口的真实记录、撤权和期限回归。
use crate::filesystem_deadline_tests::fixture;
use diskgraph_core::{Authorizer, BusinessError};
use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn management_reads_preserve_records_and_refuse_revoked_scope() {
    let (_dir, service, scope) = fixture();
    let deadline = || Instant::now() + Duration::from_secs(1);
    let job = service
        .engine()
        .control_store()
        .unwrap()
        .create_job(
            &scope,
            diskgraph_store::JobKind::Index,
            service.context.principal(),
        )
        .unwrap();
    let scopes = service.scope_tool(&json!({}), deadline()).unwrap();
    assert!(
        scopes["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["scope_id"] == scope.as_str())
    );
    let snapshots = service
        .snapshots_tool(&Some(scope.clone()), &json!({}), deadline())
        .unwrap();
    assert_eq!(
        snapshots["snapshots"][0]["snapshot_id"],
        "deadline-snapshot"
    );
    assert_eq!(snapshots["snapshots"][0]["complete"], true);
    let status = service
        .status_tool(&json!({"job_id":job.job_id}), deadline())
        .unwrap();
    assert_eq!(status["scope_id"], scope.as_str());
    assert_eq!(status["state"], "queued");
    service
        .engine()
        .control_store()
        .unwrap()
        .revoke_scope(&scope)
        .unwrap();
    assert_eq!(
        crate::business_of(
            &service
                .snapshots_tool(&Some(scope), &json!({}), deadline())
                .unwrap_err()
        ),
        BusinessError::PermissionDenied
    );
    assert_eq!(
        crate::business_of(
            &service
                .status_tool(&json!({"job_id":job.job_id}), deadline())
                .unwrap_err()
        ),
        BusinessError::PermissionDenied
    );
}

#[test]
fn management_reads_intersect_captured_capability_with_live_grants() {
    let (_dir, service, scope) = fixture();
    let policy = service.authorizer().unwrap();
    let job = service
        .engine()
        .control_store()
        .unwrap()
        .create_job(
            &scope,
            diskgraph_store::JobKind::Index,
            service.context.principal(),
        )
        .unwrap();
    {
        let mut control = service.engine().control_store().unwrap();
        control
            .revoke_grant(
                service.context.principal(),
                &diskgraph_core::Permission::MetadataRead,
                &scope,
            )
            .unwrap();
        control
            .revoke_grant(
                service.context.principal(),
                &diskgraph_core::Permission::OperationView,
                &scope,
            )
            .unwrap();
    }
    assert_eq!(
        crate::business_of(
            &service
                .engine()
                .list_snapshots_until(
                    &scope,
                    service.context.principal(),
                    &policy,
                    20,
                    0,
                    Instant::now() + Duration::from_secs(1)
                )
                .unwrap_err()
        ),
        BusinessError::PermissionDenied
    );
    assert_eq!(
        crate::business_of(
            &service
                .engine()
                .job_status_authorized_until(
                    &job.job_id,
                    service.context.principal(),
                    &policy,
                    Instant::now() + Duration::from_secs(1)
                )
                .unwrap_err()
        ),
        BusinessError::PermissionDenied
    );
}

/// 在指定锁外能力回调中实际撤权再恢复，用于证明当前请求不能忘记已提交撤权。
/// 来源：DiskGraph 原生 Rust 管理读取撤权回归契约。
struct RegrantingAuthority<'a> {
    engine: &'a diskgraph_engine::Engine,
    policy: crate::request_authorizer::RequestAuthorizer,
    scope: diskgraph_core::ScopeId,
    permission: diskgraph_core::Permission,
    calls: std::cell::Cell<usize>,
    target: usize,
}

impl Authorizer for RegrantingAuthority<'_> {
    fn decide(
        &self,
        principal: &diskgraph_core::PrincipalId,
        permission: &diskgraph_core::Permission,
        scope: &diskgraph_core::ScopeId,
    ) -> diskgraph_core::Decision {
        assert!(
            self.engine.try_control_store().unwrap().is_some(),
            "callback held control owner"
        );
        let calls = self.calls.get();
        self.calls.set(calls + 1);
        if calls == self.target {
            let mut control = self.engine.control_store().unwrap();
            control
                .revoke_grant(principal, &self.permission, &self.scope)
                .unwrap();
            let policy_version = control.policy_version().unwrap();
            control
                .upsert_grant(&diskgraph_core::Grant {
                    principal: principal.clone(),
                    permission: self.permission,
                    scope: self.scope.clone(),
                    policy_version,
                })
                .unwrap();
        }
        self.policy.decide(principal, permission, scope)
    }
    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }
    fn expires_at_unix_seconds(&self) -> Option<u64> {
        self.policy.expires_at_unix_seconds()
    }
}

#[test]
fn snapshot_read_remembers_revocation_restored_during_terminal_callback() {
    let (_dir, service, scope) = fixture();
    let native_watch = service
        .engine()
        .control_store()
        .unwrap()
        .watch_authorization_withdrawal(
            service.context.principal(),
            &scope,
            &diskgraph_core::Permission::MetadataRead,
        )
        .unwrap()
        .is_some();
    let authority = RegrantingAuthority {
        engine: service.engine(),
        policy: service.authorizer().unwrap(),
        scope: scope.clone(),
        permission: diskgraph_core::Permission::MetadataRead,
        calls: std::cell::Cell::new(0),
        target: 1,
    };
    let result = service.engine().list_snapshots_until(
        &scope,
        service.context.principal(),
        &authority,
        20,
        0,
        Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(authority.calls.get(), 2);
    assert_eq!(
        crate::business_of(&result.unwrap_err()),
        if native_watch {
            BusinessError::PermissionDenied
        } else {
            BusinessError::Conflict
        }
    );
}

#[test]
fn job_status_remembers_revocation_restored_during_capability_callback() {
    let (_dir, service, scope) = fixture();
    let job = service
        .engine()
        .control_store()
        .unwrap()
        .create_job(
            &scope,
            diskgraph_store::JobKind::Index,
            service.context.principal(),
        )
        .unwrap();
    let native_watch = service
        .engine()
        .control_store()
        .unwrap()
        .watch_authorization_withdrawal(
            service.context.principal(),
            &scope,
            &diskgraph_core::Permission::OperationView,
        )
        .unwrap()
        .is_some();
    let authority = RegrantingAuthority {
        engine: service.engine(),
        policy: service.authorizer().unwrap(),
        scope,
        permission: diskgraph_core::Permission::OperationView,
        calls: std::cell::Cell::new(0),
        target: 0,
    };
    let result = service.engine().job_status_authorized_until(
        &job.job_id,
        service.context.principal(),
        &authority,
        Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(
        crate::business_of(&result.unwrap_err()),
        if native_watch {
            BusinessError::PermissionDenied
        } else {
            BusinessError::Conflict
        }
    );
}

#[test]
fn revision_reader_remembers_revocation_restored_during_capability_callback() {
    for (target, during_consumer) in [(0, false), (1, false), (1, true)] {
        let (_dir, service, scope) = fixture();
        let native_watch = service
            .engine()
            .control_store()
            .unwrap()
            .watch_authorization_withdrawal(
                service.context.principal(),
                &scope,
                &diskgraph_core::Permission::MetadataRead,
            )
            .unwrap()
            .is_some();
        let authority = RegrantingAuthority {
            engine: service.engine(),
            policy: service.authorizer().unwrap(),
            scope,
            permission: diskgraph_core::Permission::MetadataRead,
            calls: std::cell::Cell::new(0),
            target,
        };
        let result = service.engine().with_authorized_revision_reader_until(
            "deadline-revision",
            service.context.principal(),
            &authority,
            Instant::now() + Duration::from_secs(1),
            |reader, snapshot, _| {
                if during_consumer {
                    authority.decide(
                        service.context.principal(),
                        &diskgraph_core::Permission::MetadataRead,
                        &authority.scope,
                    );
                }
                Ok(reader.snapshot(snapshot)?.id)
            },
        );
        assert_eq!(
            crate::business_of(&result.unwrap_err()),
            if native_watch {
                BusinessError::PermissionDenied
            } else {
                BusinessError::Conflict
            },
            "callback position {target}"
        );
    }
}

#[test]
fn display_reader_remembers_revocation_restored_during_capability_callback() {
    for (target, during_consumer) in [(0, false), (1, false), (1, true)] {
        let (_dir, service, scope) = fixture();
        let native_watch = service
            .engine()
            .control_store()
            .unwrap()
            .watch_authorization_withdrawal(
                service.context.principal(),
                &scope,
                &diskgraph_core::Permission::MetadataRead,
            )
            .unwrap()
            .is_some();
        let authority = RegrantingAuthority {
            engine: service.engine(),
            policy: service.authorizer().unwrap(),
            scope,
            permission: diskgraph_core::Permission::MetadataRead,
            calls: std::cell::Cell::new(0),
            target,
        };
        let result = service
            .engine()
            .with_authorized_revision_display_reader_bounded(
                "deadline-revision",
                service.context.principal(),
                &authority,
                diskgraph_core::QueryBudget {
                    deadline_ms: 1000,
                    ..diskgraph_core::QueryBudget::default()
                },
                |reader, snapshot, _| {
                    if during_consumer {
                        authority.decide(
                            service.context.principal(),
                            &diskgraph_core::Permission::MetadataRead,
                            &authority.scope,
                        );
                    }
                    reader.snapshot(snapshot)?;
                    Ok(diskgraph_engine::RevisionDisplayCompletion::Truncated)
                },
            );
        assert_eq!(
            crate::business_of(&result.unwrap_err()),
            if native_watch {
                BusinessError::PermissionDenied
            } else {
                BusinessError::Conflict
            },
            "callback position {target}"
        );
    }
}

#[test]
fn revision_admission_remembers_revocation_restored_during_capability_callback() {
    let (_dir, service, scope) = fixture();
    let native_watch = service
        .engine()
        .control_store()
        .unwrap()
        .watch_authorization_withdrawal(
            service.context.principal(),
            &scope,
            &diskgraph_core::Permission::MetadataRead,
        )
        .unwrap()
        .is_some();
    let authority = RegrantingAuthority {
        engine: service.engine(),
        policy: service.authorizer().unwrap(),
        scope,
        permission: diskgraph_core::Permission::MetadataRead,
        calls: std::cell::Cell::new(0),
        target: 0,
    };
    let result = service.engine().authorize_revision_until(
        None,
        "deadline-revision",
        service.context.principal(),
        &authority,
        Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(
        crate::business_of(&result.unwrap_err()),
        if native_watch {
            BusinessError::PermissionDenied
        } else {
            BusinessError::Conflict
        }
    );
}
