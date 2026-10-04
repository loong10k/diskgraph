//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use diskgraph_core::{Envelope, ScopeId};
use serde_json::json;

#[test]
fn with_ids_serializes_the_scope_and_revision_it_bound() {
    let scope = ScopeId::new("scope-x").unwrap();
    let revision = diskgraph_core::RevisionId::new("rev-y").unwrap();
    let envelope = Envelope::ok(json!({})).with_ids(
        Some(diskgraph_core::ServerId::new("srv").unwrap()),
        Some(scope.clone()),
        Some(revision.clone()),
    );
    let value = envelope.into_json();
    assert_eq!(value["scope_id"], scope.as_str());
    assert_eq!(value["revision_id"], revision.as_str());
}
