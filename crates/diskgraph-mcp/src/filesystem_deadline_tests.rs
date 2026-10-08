//! 使用隔离的已发布快照验证窄读期限与查询兼容性，不依赖扫描宿主部署。
use crate::{McpConfig, McpService};
use diskgraph_core::{BusinessError, ScopeId};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// 建立隔离已发布快照。参数：无；返回：保活目录、服务及真实范围。
pub(super) fn fixture() -> (tempfile::TempDir, McpService, ScopeId) {
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let scope = service
        .engine()
        .register_scope(
            dir.path(),
            service.context.principal(),
            &service.authorizer().unwrap(),
        )
        .unwrap();
    let root = json!({"id":1,"parent_id":null,"locator":{"type":"native_path","value":dir.path()},"name":"root","kind":"directory","subtree_bytes":8,"direct_bytes":0,"size_known":true,"files":2,"directories":1,"modified_unix_seconds":null,"file_identity":null,"category_hint":null,"reclaim_hint":null,"read_error":false});
    let mut nodes = vec![root.clone()];
    for (id, name, size) in [(2, "École", 3), (3, "文件", 5)] {
        let mut node = root.clone();
        node["id"] = json!(id);
        node["parent_id"] = json!(1);
        node["name"] = json!(name);
        node["kind"] = json!("file");
        node["subtree_bytes"] = json!(size);
        node["direct_bytes"] = json!(size);
        node["files"] = json!(1);
        node["directories"] = json!(0);
        node["locator"] = json!({"type":"native_path","value":dir.path().join(name)});
        nodes.push(node);
    }
    let graph: diskgraph_core::DiskGraph = serde_json::from_value(json!({"snapshot":{"id":"deadline-snapshot","root":{"type":"native_path","value":dir.path()},"volume_id":null,"captured_at_unix_ms":1,"settings":{"apparent_size":true,"follow_links":false,"include_hidden":true,"one_filesystem":true,"max_depth":null,"dedup_hardlinks":true},"coverage":{"complete":true,"unreadable_nodes":0,"depth_limited":false}},"nodes":nodes,"evidence":[]})).unwrap();
    let server = service.engine().server_id().unwrap();
    {
        let mut store =
            diskgraph_store::SqliteSnapshotStore::open(&dir.path().join("data/diskgraph.sqlite"))
                .unwrap();
        store
            .append_staging_nodes("deadline-job", &graph.nodes)
            .unwrap();
        store
            .publish_revision_owned(
                "deadline-job",
                &graph,
                "deadline-revision",
                1,
                Some((server.as_str(), scope.as_str())),
            )
            .unwrap();
    }
    (dir, service, scope)
}

fn query(
    service: &McpService,
    scope: &ScopeId,
    mode: usize,
    deadline: Instant,
) -> Result<Value, diskgraph_engine::EngineError> {
    let arguments = json!({"revision":"deadline-revision","node_id":2,"pattern":"é","limit":1});
    let scope = Some(scope.clone());
    match mode {
        0 => service.explore_tool(&scope, &json!({"revision":"deadline-revision"}), deadline),
        1 => service.search_tool(&scope, &arguments, deadline),
        2 => service.node_tool(&scope, &arguments, deadline),
        3 => service.children_tool(&scope, &arguments, deadline),
        _ => service.top_tool(&scope, &arguments, deadline),
    }
}

#[test]
fn narrow_queries_preserve_unicode_order_coverage_and_cursor() {
    let (_dir, service, scope) = fixture();
    let answers: Vec<_> = (0..5)
        .map(|mode| {
            query(
                &service,
                &scope,
                mode,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(answers[0]["children"].as_array().unwrap().len(), 2);
    assert_eq!(answers[0]["coverage"]["complete"], true);
    assert_eq!(answers[1]["items"][0]["name"], "École");
    assert_eq!(answers[2]["node"]["id"], 2);
    assert_eq!(answers[3]["items"][0]["id"], 3);
    let next = service
        .children_tool(
            &Some(scope),
            &json!({"revision":"deadline-revision","limit":1,"cursor":answers[3]["next_cursor"]}),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(next["items"][0]["id"], 2);
    assert!(next["next_cursor"].is_null());
    assert_eq!(answers[4]["items"][0]["id"], 3);
}

#[test]
fn narrow_queries_refuse_expired_original_deadline_and_revoked_scope() {
    let (_dir, service, scope) = fixture();
    for mode in 0..5 {
        assert_eq!(
            crate::business_of(
                &query(
                    &service,
                    &scope,
                    mode,
                    Instant::now() - Duration::from_millis(1)
                )
                .unwrap_err()
            ),
            BusinessError::BudgetExceeded
        );
    }
    service
        .engine()
        .control_store()
        .unwrap()
        .revoke_scope(&scope)
        .unwrap();
    for mode in 0..5 {
        assert_eq!(
            crate::business_of(
                &query(
                    &service,
                    &scope,
                    mode,
                    Instant::now() + Duration::from_secs(1)
                )
                .unwrap_err()
            ),
            BusinessError::PermissionDenied
        );
    }
}

fn terminal_reply_does_not_reacquire_unbounded_policy(history: bool) {
    let (_dir, service, _scope) = fixture();
    let captured = service.authorizer().unwrap();
    let owner = service.engine().control_store().unwrap();
    let request = service.clone();
    let (finished, result) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let envelope =
            json!({"revision_id":"deadline-revision","data":{"complete":true},"truncated":false});
        let deadline = Instant::now() - Duration::from_millis(1);
        let outcome = if history {
            crate::snapshot_reply::finish(
                &request,
                envelope,
                &["deadline-revision", "deadline-revision"],
                deadline,
                &captured,
            )
        } else {
            crate::relation_reply::finish(&request, envelope, deadline, &captured)
        };
        finished
            .send(outcome.err().map(|error| crate::business_of(&error)))
            .unwrap();
    });
    let bounded = result.recv_timeout(Duration::from_millis(300));
    drop(owner);
    worker.join().unwrap();
    assert!(
        matches!(bounded, Ok(Some(BusinessError::BudgetExceeded))),
        "terminal reply waited for control release: {bounded:?}"
    );
}

#[test]
fn relation_reply_refuses_occupied_control_without_unbounded_policy_read() {
    terminal_reply_does_not_reacquire_unbounded_policy(false);
}
#[test]
fn history_reply_refuses_occupied_control_without_unbounded_policy_read() {
    terminal_reply_does_not_reacquire_unbounded_policy(true);
}

fn finish_with_captured_policy(
    service: &McpService,
    history: bool,
    policy: &dyn diskgraph_core::Authorizer,
) -> Result<(Value, String), diskgraph_engine::EngineError> {
    let envelope =
        json!({"revision_id":"deadline-revision","data":{"complete":true},"truncated":false});
    let deadline = Instant::now() - Duration::from_millis(1);
    if history {
        crate::snapshot_reply::finish(
            service,
            envelope,
            &["deadline-revision", "deadline-revision"],
            deadline,
            policy,
        )
    } else {
        crate::relation_reply::finish(service, envelope, deadline, policy)
    }
}

#[test]
fn expired_reply_preserves_partial_diagnostic_with_live_captured_capability() {
    let (_dir, service, _scope) = fixture();
    let policy = service.authorizer().unwrap();
    for history in [false, true] {
        let (envelope, text) = finish_with_captured_policy(&service, history, &policy).unwrap();
        assert_eq!(envelope["data"]["complete"], false);
        assert_eq!(envelope["data"]["truncated"], "deadline");
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), envelope);
    }
}

#[test]
fn captured_reply_capability_cannot_outlive_persistent_grant_revocation() {
    let (_dir, service, scope) = fixture();
    let policy = service.authorizer().unwrap();
    service
        .engine()
        .control_store()
        .unwrap()
        .revoke_grant(
            service.context.principal(),
            &diskgraph_core::Permission::MetadataRead,
            &scope,
        )
        .unwrap();
    for history in [false, true] {
        assert_eq!(
            crate::business_of(
                &finish_with_captured_policy(&service, history, &policy).unwrap_err()
            ),
            BusinessError::PermissionDenied
        );
    }
}
