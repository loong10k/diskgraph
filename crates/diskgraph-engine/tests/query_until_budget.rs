//! D24 新 until 入口的实际期限、终态撤权及未知大小回归。

mod query_request_budget {
    pub(super) mod callback_authorizer;
    pub(super) mod fixture;
}

use diskgraph_core::{
    BusinessError, Permission, PolicyAuthorizer, QueryBudget, ScopeId, SyncMethod,
};
use diskgraph_engine::EngineError;
use diskgraph_engine::content::{ConservativeProbe, InspectionRequest};
use diskgraph_store::{ControlStore, SqliteSnapshotStore, StoreError};
use query_request_budget::callback_authorizer::CallbackAuthorizer;
use query_request_budget::fixture::Fixture;
use std::path::Path;
use std::time::{Duration, Instant};

fn is_budget_error(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::Business(BusinessError::BudgetExceeded | BusinessError::Timeout)
            | EngineError::Store(StoreError::BudgetExceeded)
    )
}

fn other_revision(f: &Fixture) -> (ScopeId, String, PolicyAuthorizer) {
    let root = f.directory.path().join("other-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a"), b"abcdefgh").unwrap();
    let scope = f
        .engine
        .register_scope(
            &root.canonicalize().unwrap(),
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let policy = f.engine.policy_authorizer().unwrap();
    let job = f.engine.index_scope(&scope, &f.principal, &policy).unwrap();
    f.engine
        .run_job(&job.job_id, "other-history-budget-owner")
        .unwrap();
    let revision = f.engine.latest_revision(&scope).unwrap().unwrap();
    (scope, revision, policy)
}

#[test]
fn all_until_entry_points_refuse_an_already_expired_request() {
    let f = Fixture::new(true);
    let expired = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
    let budget = QueryBudget::default();
    let results = [
        f.engine
            .tree_view_until(
                &f.scope,
                &f.revision,
                &f.principal,
                &f.policy,
                2,
                0,
                budget,
                expired,
            )
            .map(|_| ()),
        f.engine
            .compare_revisions_until(
                &f.revision,
                &f.revision,
                0,
                budget,
                &f.principal,
                &f.policy,
                expired,
            )
            .map(|_| ()),
        f.engine
            .growth_between_until(
                &f.revision,
                &f.revision,
                Path::new("a"),
                budget,
                &f.principal,
                &f.policy,
                expired,
            )
            .map(|_| ()),
        f.engine
            .revision_changes_until(
                &f.revision,
                &f.revision,
                budget,
                &f.principal,
                &f.policy,
                expired,
            )
            .map(|_| ()),
        f.engine
            .sync_plan_until(
                &f.revision,
                &f.revision,
                SyncMethod::Update,
                0,
                budget,
                &f.principal,
                &f.policy,
                expired,
            )
            .map(|_| ()),
    ];
    // 全部入口都实际执行，不能首个失败遮住后续入口。
    for (index, result) in results.into_iter().enumerate() {
        assert!(
            result.as_ref().is_err_and(is_budget_error),
            "entry {index}: {result:?}"
        );
    }
}

#[test]
fn typed_history_deadline_keeps_the_existing_more_than_one_second_contract() {
    let f = Fixture::new(true);
    let budget = QueryBudget {
        deadline_ms: 5_000,
        ..QueryBudget::default()
    };
    let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
    let report = f
        .engine
        .compare_revisions_until(
            &f.revision,
            &f.revision,
            0,
            budget,
            &f.principal,
            &f.policy,
            deadline,
        )
        .unwrap();
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.truncated, None);
    // generic reader 的原始 1–1000ms 参数契约不因 typed until 扩展而改变。
    assert!(matches!(
        f.engine.with_authorized_revision_reader(
            &f.revision,
            &f.principal,
            &f.policy,
            1_001,
            |_, _, _| Ok(())
        ),
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));
}

#[test]
fn terminal_scope_revocation_is_checked_after_the_encoded_history() {
    let f = Fixture::new(false);
    let path = f.directory.path().join("data/diskgraph-control.sqlite");
    let scope = f.scope.clone();
    let policy = CallbackAuthorizer::new(f.policy.clone(), move |call| {
        if call == 5 {
            // 双侧初检及编码前双侧末检已完成；实际独立连接在编码后撤销。
            ControlStore::open(&path)
                .unwrap()
                .revoke_scope(&scope)
                .unwrap();
        }
    });
    let result = f.engine.compare_revisions_until(
        &f.revision,
        &f.revision,
        0,
        QueryBudget::default(),
        &f.principal,
        &policy,
        Instant::now().checked_add(Duration::from_secs(5)).unwrap(),
    );
    assert!(policy.calls.get() >= 5, "encoding terminal check must run");
    assert!(f.engine.scope(&f.scope).unwrap().revoked);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "encoded revoked history leaked; error={:?}",
        result.err()
    );
}

#[test]
fn terminal_single_metadata_grant_revocation_refuses_the_encoded_history() {
    let f = Fixture::new(true);
    let path = f.directory.path().join("data/diskgraph-control.sqlite");
    let scope = f.scope.clone();
    let principal = f.principal.clone();
    let policy = CallbackAuthorizer::new(f.policy.clone(), move |call| {
        if call == 5 {
            ControlStore::open(&path)
                .unwrap()
                .revoke_grant(&principal, &Permission::MetadataRead, &scope)
                .unwrap();
        }
    });
    let result = f.engine.revision_changes_until(
        &f.revision,
        &f.revision,
        QueryBudget::default(),
        &f.principal,
        &policy,
        Instant::now().checked_add(Duration::from_secs(5)).unwrap(),
    );
    assert!(policy.calls.get() >= 5, "encoding terminal check must run");
    let control = f.engine.control_store().unwrap();
    assert_eq!(
        control
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(false)
    );
    assert_eq!(
        control
            .live_permission(&f.principal, &Permission::IndexWrite, &f.scope)
            .unwrap(),
        Some(true)
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "encoded revoked history leaked: {result:?}"
    );
}

#[test]
fn final_right_authorizer_cannot_revoke_the_already_checked_left_scope() {
    let f = Fixture::new(true);
    let (right_scope, right_revision, current_policy) = other_revision(&f);
    let path = f.directory.path().join("data/diskgraph-control.sqlite");
    let left_scope = f.scope.clone();
    let callback = CallbackAuthorizer::new(current_policy, move |call| {
        if call == 6 {
            ControlStore::open(&path)
                .unwrap()
                .revoke_scope(&left_scope)
                .unwrap();
        }
    });
    let result = f.engine.revision_changes_until(
        &f.revision,
        &right_revision,
        QueryBudget::default(),
        &f.principal,
        &callback,
        Instant::now().checked_add(Duration::from_secs(5)).unwrap(),
    );
    assert!(callback.calls.get() >= 6, "both final callbacks must run");
    let control = f.engine.control_store().unwrap();
    assert!(control.scope(&f.scope).unwrap().revoked);
    assert_eq!(
        control
            .live_permission(&f.principal, &Permission::MetadataRead, &right_scope)
            .unwrap(),
        Some(true)
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "left revoked after left check: {result:?}"
    );
}

#[test]
fn final_right_authorizer_cannot_revoke_the_already_checked_left_grant() {
    let f = Fixture::new(true);
    let (right_scope, right_revision, current_policy) = other_revision(&f);
    let path = f.directory.path().join("data/diskgraph-control.sqlite");
    let left_scope = f.scope.clone();
    let principal = f.principal.clone();
    let callback = CallbackAuthorizer::new(current_policy, move |call| {
        if call == 6 {
            ControlStore::open(&path)
                .unwrap()
                .revoke_grant(&principal, &Permission::MetadataRead, &left_scope)
                .unwrap();
        }
    });
    let result = f.engine.revision_changes_until(
        &f.revision,
        &right_revision,
        QueryBudget::default(),
        &f.principal,
        &callback,
        Instant::now().checked_add(Duration::from_secs(5)).unwrap(),
    );
    assert!(callback.calls.get() >= 6, "both final callbacks must run");
    let control = f.engine.control_store().unwrap();
    assert_eq!(
        control
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(false)
    );
    assert_eq!(
        control
            .live_permission(&f.principal, &Permission::MetadataRead, &right_scope)
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        control
            .live_permission(&f.principal, &Permission::IndexWrite, &f.scope)
            .unwrap(),
        Some(true)
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "left grant revoked after left check: {result:?}"
    );
}

#[test]
fn a_partial_history_cannot_become_a_usable_sync_plan() {
    let f = Fixture::new(true);
    let result = f.engine.sync_plan_until(
        &f.revision,
        &f.revision,
        SyncMethod::Update,
        0,
        QueryBudget {
            max_nodes: 4,
            ..QueryBudget::default()
        },
        &f.principal,
        &f.policy,
        Instant::now().checked_add(Duration::from_secs(5)).unwrap(),
    );
    assert!(
        result.as_ref().is_err_and(is_budget_error),
        "partial plan escaped: {result:?}"
    );
}

#[test]
fn history_keeps_the_completed_pair_without_decoding_the_next_corrupt_pair() {
    let f = Fixture::new(true);
    assert_eq!(f.db.execute("UPDATE nodes SET locator_key=json_set(locator_key,'$.type','invalid_fixture_kind') WHERE snapshot_id=?1 AND name='b'", [&f.snapshot]).unwrap(), 1);
    assert!(
        f.engine
            .compare_revisions_bounded(&f.revision, &f.revision, 0, QueryBudget::default())
            .is_err(),
        "unrestricted positive control must decode the corrupt b row"
    );
    // 两个根和 a 的双侧记录恰好耗尽四次解码，仍须保留已经完整比较的 a。
    let report = f
        .engine
        .compare_revisions_bounded(
            &f.revision,
            &f.revision,
            0,
            QueryBudget {
                max_nodes: 4,
                ..QueryBudget::default()
            },
        )
        .unwrap();
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].path, "a");
    assert_eq!(report.truncated, Some("node_limit"));
    assert_eq!(report.summary.same, 1);
}

#[test]
fn read_preparation_cannot_open_content_after_the_shared_deadline() {
    let f = Fixture::new(true);
    f.engine
        .set_content_read(&f.scope, &f.principal, true)
        .unwrap();
    let policy = CallbackAuthorizer::new(f.engine.policy_authorizer().unwrap(), |call| {
        if call == 1 {
            std::thread::sleep(Duration::from_millis(40));
        }
    });
    let path = f.directory.path().join("root/a").canonicalize().unwrap();
    let result = f.engine.read_bounded_until(
        &InspectionRequest {
            scope_id: &f.scope,
            principal: &f.principal,
            path: &path,
            offset: 0,
            max_bytes: 8,
            cancel: None,
            chunk_bytes: 8,
        },
        &ConservativeProbe,
        &policy,
        Instant::now()
            .checked_add(Duration::from_millis(1))
            .unwrap(),
    );
    assert!(
        matches!(result, Err(EngineError::Business(BusinessError::Timeout))),
        "late content preparation produced bytes: {result:?}"
    );
}

#[test]
fn published_unknown_tree_size_keeps_its_diagnostic_and_numeric_minimum_contract() {
    let f = Fixture::new(true);
    let reader = f.engine.revision_reader().unwrap();
    let mut graph = reader.load(&f.snapshot).unwrap();
    drop(reader);
    graph.snapshot.id = "published-unknown-tree".to_owned();
    let node = graph
        .nodes
        .iter_mut()
        .find(|node| node.name == "a")
        .unwrap();
    node.size_known = false;
    node.subtree_bytes = 0;
    node.direct_bytes = 0;
    let mut writer =
        SqliteSnapshotStore::open(&f.directory.path().join("data/diskgraph.sqlite")).unwrap();
    writer
        .publish_revision("unknown-tree-test", &graph, "unknown-tree-revision", 1)
        .unwrap();
    drop(writer);
    let tree = f
        .engine
        .tree_view_bounded("unknown-tree-revision", 2, 0, QueryBudget::default())
        .unwrap();
    let children = tree.root["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);
    let unknown = children.iter().find(|node| node["name"] == "a").unwrap();
    assert_eq!(
        unknown["size_known"], false,
        "unknown cannot masquerade as measured zero"
    );
    assert_eq!(
        unknown["size_bytes"], 0,
        "existing numeric field stays compatible"
    );
    let filtered = f
        .engine
        .tree_view_bounded("unknown-tree-revision", 2, 1, QueryBudget::default())
        .unwrap();
    assert_eq!(filtered.root["children"].as_array().unwrap().len(), 1);
    assert_eq!(
        filtered.root["children"][0]["name"], "b",
        "minimum keeps existing numeric filtering rather than classifying unknown as measured zero"
    );
}
