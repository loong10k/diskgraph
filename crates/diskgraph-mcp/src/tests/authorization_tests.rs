//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use super::support::{call, cargo_project, seed, service, structured};
use crate::{auth, protocol::ToolProfile};
use diskgraph_core::ScopeId;
use serde_json::json;

#[test]
fn impact_requires_the_revision_owners_grant_even_when_a_different_scope_is_supplied() {
    use diskgraph_core::{Grant, Permission};

    let (mut service, data) = service(ToolProfile::ReadFull, "impact-scope");
    let (first, first_root) = cargo_project("impact-a");
    let (second, second_root) = cargo_project("impact-b");
    let scope_a = seed(&mut service, &first_root);
    let scope_b = seed(&mut service, &second_root);
    let revision_b = service
        .engine()
        .latest_revision(&ScopeId::new(scope_b.clone()).unwrap())
        .unwrap()
        .unwrap();
    let graph_b = service.engine().load_revision(&revision_b).unwrap();
    let target = graph_b
        .nodes
        .iter()
        .find(|node| node.name == "target")
        .unwrap();
    let auth = auth::Authenticator::new(auth::AuthConfig::single("issuer", "aud", b"test-key"));
    let token = auth::TokenMinter::new(b"test-key").mint(&auth::TokenClaims {
        issuer: "issuer".into(),
        audience: "aud".into(),
        subject: "only-a".into(),
        expires_at_unix_seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 300,
        scope: Some("metadata:read".into()),
    });
    let identity = auth.authenticate(Some(&token)).unwrap();
    let mut control =
        diskgraph_store::ControlStore::open(&data.path().join("data/diskgraph-control.sqlite"))
            .unwrap();
    control
        .upsert_grant(&Grant {
            principal: identity.principal.clone(),
            permission: Permission::MetadataRead,
            scope: ScopeId::new(scope_a.clone()).unwrap(),
            policy_version: control.policy_version().unwrap(),
        })
        .unwrap();
    let mut remote = service.for_identity(&identity);
    let response = call(
        &mut remote,
        "diskgraph_impact",
        json!({"scope":scope_a,"revision":revision_b,"entity":format!("resource-{}", target.id)}),
    );
    assert_eq!(
        response["error"]["data"]["business_code"], "permission_denied",
        "{response}"
    );
    let mismatch = call(
        &mut service,
        "diskgraph_impact",
        json!({"scope":scope_a,"revision":revision_b,"entity":format!("resource-{}",target.id)}),
    );
    assert_eq!(
        mismatch["error"]["data"]["business_code"],
        "permission_denied"
    );
    let authorized = call(
        &mut service,
        "diskgraph_impact",
        json!({"revision":revision_b,"entity":format!("resource-{}",target.id)}),
    );
    assert_eq!(structured(&authorized)["scope_id"], scope_b);
    assert_eq!(structured(&authorized)["revision_id"], revision_b);
    drop(first);
    drop(second);
}
