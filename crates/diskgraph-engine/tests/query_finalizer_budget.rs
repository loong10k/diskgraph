//! 成组回复终检必须在全部能力回调后复读所有持久授权。

mod query_request_budget {
    pub(super) mod callback_authorizer;
    pub(super) mod fixture;
}

use diskgraph_core::{BusinessError, Permission};
use diskgraph_engine::EngineError;
use diskgraph_store::ControlStore;
use query_request_budget::callback_authorizer::CallbackAuthorizer;
use query_request_budget::fixture::Fixture;
use std::time::{Duration, Instant};

fn assert_cross_scope_reply_refused(revoke_grant: bool) {
    let f = Fixture::new(true);
    // 直接 SQL 正控制：请求的左 revision 确实绑定该快照及三个真实节点。
    let (snapshot, nodes): (String, i64) = f.db.query_row(
        "SELECT r.snapshot_id,COUNT(n.id) FROM graph_revisions r JOIN nodes n ON n.snapshot_id=r.snapshot_id WHERE r.revision_id=?1 GROUP BY r.snapshot_id",
        [&f.revision], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(snapshot, f.snapshot);
    assert_eq!(nodes, 3);
    let other = f.directory.path().join("other-root");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("a"), b"abcdefgh").unwrap();
    let other_scope = f
        .engine
        .register_scope(
            &other.canonicalize().unwrap(),
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let current = f.engine.policy_authorizer().unwrap();
    let job = f
        .engine
        .index_scope(&other_scope, &f.principal, &current)
        .unwrap();
    f.engine
        .run_job(&job.job_id, "reply-finalizer-owner")
        .unwrap();
    let other_revision = f.engine.latest_revision(&other_scope).unwrap().unwrap();
    let control_path = f.directory.path().join("data/diskgraph-control.sqlite");
    let left_scope = f.scope.clone();
    let principal = f.principal.clone();
    let callback = CallbackAuthorizer::new(current, move |call| {
        if call == 2 {
            let mut control = ControlStore::open(&control_path).unwrap();
            if revoke_grant {
                control
                    .revoke_grant(&principal, &Permission::MetadataRead, &left_scope)
                    .unwrap();
            } else {
                control.revoke_scope(&left_scope).unwrap();
            }
        }
    });
    let result = f.engine.finalize_revisions_read_until(
        &[&f.revision, &other_revision],
        &f.principal,
        &callback,
        Instant::now().checked_add(Duration::from_secs(5)).unwrap(),
    );
    assert_eq!(
        callback.calls.get(),
        2,
        "pure persisted checks must not invoke callbacks again"
    );
    let control = f.engine.control_store().unwrap();
    assert_eq!(
        control
            .live_permission(&f.principal, &Permission::MetadataRead, &other_scope)
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        control
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(false)
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "group reply returned data after left revocation: {result:?}"
    );
}

#[test]
fn grouped_reply_refuses_scope_revoked_by_the_last_authorizer() {
    assert_cross_scope_reply_refused(false);
}

#[test]
fn grouped_reply_refuses_one_grant_revoked_by_the_last_authorizer() {
    assert_cross_scope_reply_refused(true);
}

#[test]
fn empty_reply_revision_set_cannot_be_considered_authorized() {
    let f = Fixture::new(true);
    let result = f.engine.finalize_revisions_read_until(
        &[],
        &f.principal,
        &f.policy,
        Instant::now().checked_add(Duration::from_secs(5)).unwrap(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));
}
