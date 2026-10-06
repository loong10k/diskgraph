//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use super::query_revision_fixture::QueryRevisionFixture;
use super::support::{call, cargo_project, payload, seed, service, structured};
use crate::McpService;
use crate::protocol::ToolProfile;
use diskgraph_core::ScopeId;
use serde_json::json;
use std::path::Path;

#[test]
fn explicit_revision_and_node_id_select_historical_data() {
    explicit_revision_and_node_id_select_historical_data_behavior(seed);
}

#[test]
fn imported_explicit_revision_and_node_id_select_historical_data() {
    explicit_revision_and_node_id_select_historical_data_behavior(QueryRevisionFixture::publish);
}

fn explicit_revision_and_node_id_select_historical_data_behavior(
    seed: fn(&mut McpService, &Path) -> String,
) {
    let (mut service, _keep) = service(ToolProfile::ReadFull, "historical-node");
    let (_project, root) = cargo_project("historical-node");
    let scope = seed(&mut service, &root);
    let scope_id = ScopeId::new(scope.clone()).unwrap();
    let before = service.engine.latest_revision(&scope_id).unwrap().unwrap();
    let graph = service.engine.load_revision(&before).unwrap();
    let target = graph
        .nodes
        .iter()
        .find(|node| node.name == "target")
        .unwrap();
    std::fs::write(root.join("new.txt"), "new revision").unwrap();
    seed(&mut service, &root);
    assert_ne!(
        service.engine.latest_revision(&scope_id).unwrap().unwrap(),
        before
    );
    let node = call(
        &mut service,
        "diskgraph_node",
        json!({"revision":before,"node_id":target.id}),
    );
    assert_eq!(payload(&node)["node"]["id"], target.id, "{node}");
    assert_eq!(structured(&node)["revision_id"], before);
    for tool in [
        "diskgraph_search",
        "diskgraph_top",
        "diskgraph_children",
        "diskgraph_explore",
    ] {
        let mut args = json!({"revision":before});
        if tool == "diskgraph_search" {
            args["pattern"] = json!("new.txt");
        }
        let response = call(&mut service, tool, args);
        assert_eq!(structured(&response)["revision_id"], before, "{response}");
        assert!(
            !payload(&response).to_string().contains("new.txt"),
            "{response}"
        );
    }
    let other = root.parent().unwrap().join("other");
    std::fs::create_dir(&other).unwrap();
    let other_scope = seed(&mut service, &other);
    let rejected = call(
        &mut service,
        "diskgraph_node",
        json!({"scope":other_scope,"revision":before,"node_id":target.id}),
    );
    assert_eq!(
        rejected["error"]["data"]["business_code"],
        "permission_denied"
    );
}

#[test]
fn queries_answer_with_envelopes_and_unknown_scopes_refuse_honestly() {
    queries_answer_with_envelopes_and_unknown_scopes_refuse_honestly_behavior(seed);
}

#[test]
fn imported_queries_answer_with_envelopes_and_unknown_scopes_refuse_honestly() {
    queries_answer_with_envelopes_and_unknown_scopes_refuse_honestly_behavior(
        QueryRevisionFixture::publish,
    );
}

fn queries_answer_with_envelopes_and_unknown_scopes_refuse_honestly_behavior(
    seed: fn(&mut McpService, &Path) -> String,
) {
    let (mut service, _keep) = service(ToolProfile::ReadFull, "queries");
    let (project, root) = cargo_project("queries");
    let scope = seed(&mut service, &root);

    let top = call(&mut service, "diskgraph_top", json!({"scope": scope}));
    let envelope = structured(&top);
    assert_eq!(envelope["api_version"], 2);
    assert_eq!(envelope["ok"], true);
    assert!(envelope["server_id"].is_string());
    let data = payload(&top);
    assert!(!data["items"].as_array().unwrap().is_empty());

    // A scope that was never indexed is `not_indexed`, not an empty success.
    let missing = call(
        &mut service,
        "diskgraph_top",
        json!({"scope": "scope-does-not-exist"}),
    );
    assert_eq!(missing["error"]["data"]["business_code"], "not_found");

    // A registered but never indexed scope reports `not_indexed`.
    let empty = project.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let empty_scope = service
        .engine()
        .register_scope(
            &empty,
            service.context.principal(),
            &service.authorizer().unwrap(),
        )
        .unwrap();
    let unindexed = call(
        &mut service,
        "diskgraph_top",
        json!({"scope": empty_scope.as_str()}),
    );
    assert_eq!(unindexed["error"]["data"]["business_code"], "not_indexed");
    drop(project);
}

#[test]
fn two_scopes_stay_isolated_and_reuse_their_own_revisions() {
    two_scopes_stay_isolated_and_reuse_their_own_revisions_behavior(seed);
}

#[test]
fn imported_two_scopes_stay_isolated_and_reuse_their_own_revisions() {
    two_scopes_stay_isolated_and_reuse_their_own_revisions_behavior(QueryRevisionFixture::publish);
}

fn two_scopes_stay_isolated_and_reuse_their_own_revisions_behavior(
    seed: fn(&mut McpService, &Path) -> String,
) {
    let (mut service, _keep) = service(ToolProfile::ReadFull, "isolation");
    let (first, first_root) = cargo_project("isolation-a");
    let (second, second_root) = cargo_project("isolation-b");
    let scope_a = seed(&mut service, &first_root);
    let scope_b = seed(&mut service, &second_root);
    assert_ne!(scope_a, scope_b);

    let revision_a = service
        .engine()
        .latest_revision(&ScopeId::new(scope_a.clone()).unwrap())
        .unwrap()
        .unwrap();
    let revision_b = service
        .engine()
        .latest_revision(&ScopeId::new(scope_b.clone()).unwrap())
        .unwrap()
        .unwrap();
    assert_ne!(revision_a, revision_b, "each scope keeps its own revision");

    // A query bound to one scope never returns the other's resources.
    let top_a = call(&mut service, "diskgraph_top", json!({"scope": scope_a}));
    let items_a = payload(&top_a)["items"].as_array().unwrap().len();
    let top_b = call(&mut service, "diskgraph_top", json!({"scope": scope_b}));
    let items_b = payload(&top_b)["items"].as_array().unwrap().len();
    assert!(items_a > 0 && items_b > 0);
    assert_eq!(
        structured(&top_a)["scope_id"],
        scope_a.as_str(),
        "the response must state which scope answered"
    );
    assert_eq!(structured(&top_b)["scope_id"], scope_b.as_str());
    // Each response is bound to its own published revision.
    assert_eq!(structured(&top_a)["revision_id"], revision_a.as_str());
    assert_ne!(
        structured(&top_a)["revision_id"],
        structured(&top_b)["revision_id"]
    );

    // Repeating the query reuses the same revision: no new publication.
    let again = call(&mut service, "diskgraph_top", json!({"scope": scope_a}));
    assert_eq!(payload(&again)["data"], payload(&top_a)["data"]);
    assert_eq!(payload(&again)["items"], payload(&top_a)["items"]);
    assert_eq!(structured(&again)["revision_id"], revision_a.as_str());
    drop(first);
    drop(second);
}
