//! 目录游标行为回归：通过实际 MCP 分发，而非仅测试游标序列化。
use diskgraph_core::{PrincipalId, ScopeId};
use serde_json::{Value, json};

use crate::{McpConfig, McpService, STDIO_PRINCIPAL, protocol};

fn fixture() -> (McpService, tempfile::TempDir, String) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("文件 é 空格");
    std::fs::create_dir(&root).unwrap();
    for name in ["a", "z", "é-ß", "文件"] {
        std::fs::write(root.join(name), "same-size").unwrap();
    }
    let service = McpService::open(McpConfig {
        data_dir: directory.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
    let engine = service.engine();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "fixture").unwrap();
    (service, directory, scope.as_str().to_owned())
}

fn call(service: &mut McpService, arguments: Value) -> Value {
    service.handle(&protocol::Request {
        id: json!(1),
        method: "tools/call".into(),
        params: json!({"name":"diskgraph_children","arguments":arguments}),
    })
}

#[test]
fn directory_cursor_resumes_in_stable_order_with_offset_compatibility() {
    let (mut service, _directory, scope) = fixture();
    let complete = call(&mut service, json!({"scope":scope,"limit":100}));
    let expected = complete["result"]["structuredContent"]["data"]["items"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(expected.len(), 4);
    let mut args = json!({"scope":scope,"limit":1});
    for (index, item) in expected.iter().enumerate() {
        let page = call(&mut service, args.clone());
        let data = &page["result"]["structuredContent"]["data"];
        assert_eq!(data["items"][0]["id"], item["id"], "{page}");
        assert_eq!(data["items"].as_array().unwrap().len(), 1);
        if index + 1 < expected.len() {
            let cursor = data["next_cursor"]
                .as_str()
                .expect("directory page must offer a keyset cursor");
            assert_eq!(data["next_offset"], (index + 1) as u64);
            // cursor 的位置具有优先权，offset 不能迫使它重复跳过前面的节点。
            args["cursor"] = json!(cursor);
            args["offset"] = json!(999_999);
        } else {
            assert!(data["next_cursor"].is_null());
            assert!(data["next_offset"].is_null());
        }
    }
    let offset = call(&mut service, json!({"scope":scope,"limit":1,"offset":2}));
    let data = &offset["result"]["structuredContent"]["data"];
    assert_eq!(data["items"][0]["id"], expected[2]["id"]);
    assert_eq!(data["next_offset"], 3);
    assert!(data["next_cursor"].is_string());
    let revision = service
        .engine()
        .latest_revision(&ScopeId::new(scope).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        offset["result"]["structuredContent"]["revision_id"],
        revision
    );
}

#[test]
fn directory_cursor_rejects_changed_parent_filter_and_policy() {
    let (mut service, _directory, scope) = fixture();
    let first = call(&mut service, json!({"scope":scope,"limit":1}));
    let cursor = first["result"]["structuredContent"]["data"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    for args in [
        json!({"scope":scope,"cursor":cursor,"parent_id":999}),
        json!({"scope":scope,"cursor":cursor,"min_bytes":1}),
        json!({"scope":scope,"cursor":"obsolete-offset-cursor"}),
    ] {
        let result = call(&mut service, args);
        assert_eq!(
            result["error"]["data"]["business_code"], "invalid_argument",
            "{result}"
        );
    }
    // 同一 scope 的两个合法主体也不能复用彼此的游标。
    let bob = PrincipalId::new("bob").unwrap();
    service
        .engine()
        .control_store()
        .unwrap()
        .upsert_grant(&diskgraph_core::Grant {
            principal: bob.clone(),
            permission: diskgraph_core::Permission::MetadataRead,
            scope: ScopeId::new(scope.clone()).unwrap(),
            policy_version: 1,
        })
        .unwrap();
    let mut request = service.for_identity(&crate::auth::AuthenticatedPrincipal {
        principal: bob,
        issuer: "fixture".into(),
        permissions: vec![diskgraph_core::Permission::MetadataRead],
        expires_at_unix_seconds: u64::MAX,
    });
    let other_subject = call(&mut request, json!({"scope":scope,"cursor":cursor}));
    assert_eq!(
        other_subject["error"]["data"]["business_code"],
        "invalid_argument"
    );
    let principal = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
    service
        .engine()
        .publish_policy_version(
            2,
            &principal,
            &service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    let expired = call(&mut service, json!({"scope":scope,"cursor":cursor}));
    assert_eq!(
        expired["error"]["data"]["business_code"],
        "invalid_argument"
    );
    let fresh = call(
        &mut service,
        json!({"scope":scope,"limit":1,"format":"treemap"}),
    );
    assert!(fresh["result"]["structuredContent"]["data"]["next_cursor"].is_string());
}

#[test]
fn directory_cursor_resumes_after_response_byte_truncation() {
    let (mut service, directory, scope) = fixture();
    let root = directory.path().join("文件 é 空格");
    for index in 0..80 {
        std::fs::write(
            root.join(format!("{index:03}-{}", "x".repeat(120))),
            "same-size",
        )
        .unwrap();
    }
    let scope_id = ScopeId::new(scope.clone()).unwrap();
    let principal = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
    let job = service
        .engine()
        .index_scope(
            &scope_id,
            &principal,
            &service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    service.engine().run_job(&job.job_id, "bytes").unwrap();
    let revision = service
        .engine()
        .latest_revision(&scope_id)
        .unwrap()
        .unwrap();
    let (all, _, _) = service
        .engine()
        .revision_children_page(&revision, 1, None, 0, 100)
        .unwrap();
    let mut found = Vec::new();
    let mut args = json!({"scope":scope,"limit":100});
    let mut truncated = false;
    for _ in 0..5 {
        let page = call(&mut service, args.clone());
        let data = &page["result"]["structuredContent"]["data"];
        assert!(
            serde_json::to_vec(data).unwrap().len()
                < diskgraph_core::QueryBudget::default().max_response_bytes,
            "{page}"
        );
        truncated |= data["truncated"].as_str() == Some("response_byte_limit");
        found.extend(
            data["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node["id"].as_u64().unwrap()),
        );
        match data["next_cursor"].as_str() {
            Some(cursor) => args["cursor"] = json!(cursor),
            None => break,
        }
    }
    assert!(truncated, "fixture must exceed one response page");
    assert_eq!(found, all.iter().map(|node| node.id).collect::<Vec<_>>());
}
