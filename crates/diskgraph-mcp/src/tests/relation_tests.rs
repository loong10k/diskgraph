//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use super::support::{call, cargo_project, payload, seed, service, structured};
use crate::protocol::ToolProfile;
use diskgraph_core::{QueryBudget, ScopeId};
use serde_json::{Value, json};

#[test]
fn explain_returns_typed_evidence_for_a_cargo_project() {
    let (mut service, _keep) = service(ToolProfile::ReadFull, "explain");
    let (project, root) = cargo_project("explain");
    let scope = seed(&mut service, &root);
    let revision = service
        .engine()
        .latest_revision(&ScopeId::new(scope.clone()).unwrap())
        .unwrap()
        .unwrap();
    let graph = service.engine().load_revision(&revision).unwrap();
    let target = graph
        .nodes
        .iter()
        .find(|node| node.name == "target")
        .unwrap();

    let response = call(
        &mut service,
        "diskgraph_explain",
        json!({"revision": revision, "entity": format!("resource-{}", target.id)}),
    );
    let data = payload(&response);
    let edges = data["edges"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .any(|edge| edge["relation"] == "owned_by_project")
    );
    assert!(!data["evidence"].as_array().unwrap().is_empty());
    drop(project);
}

#[test]
fn relation_queries_bound_decoding_and_report_continuation() {
    let (mut service, data) = service(ToolProfile::ReadFull, "relation-page");
    let (_project, root) = cargo_project("relation-page");
    let scope = seed(&mut service, &root);
    let revision = service
        .engine
        .latest_revision(&ScopeId::new(scope.clone()).unwrap())
        .unwrap()
        .unwrap();
    let graph = service.engine.load_revision(&revision).unwrap();
    let target = graph
        .nodes
        .iter()
        .find(|node| node.name == "target")
        .unwrap();
    let entity = format!("resource-{}", target.id);
    let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
    let mut edge: Value = serde_json::from_str(
        &db.query_row(
            "SELECT edge_json FROM relations WHERE source_entity_id = ?1 LIMIT 1",
            [&entity],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap();
    for n in 0..501 {
        let id = format!("fixture-{n:04}");
        edge["edge_id"] = json!(id);
        db.execute("INSERT INTO relations (snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![graph.snapshot.id,id,entity,edge["target_entity_id"].as_str().unwrap(),edge["relation"].as_str().unwrap(),edge.to_string()]).unwrap();
    }
    // 不属于首屏的损坏记录必须不被解码，页大小限制实际读取工作量。
    db.execute(
        "UPDATE relations SET edge_json = 'invalid JSON' WHERE edge_id = 'fixture-0500'",
        [],
    )
    .unwrap();
    for tool in ["diskgraph_related", "diskgraph_explain"] {
        let response = call(
            &mut service,
            tool,
            json!({"revision":revision,"entity":entity,"limit":1}),
        );
        assert_eq!(
            payload(&response)["edges"].as_array().unwrap().len(),
            1,
            "{response}"
        );
        assert_eq!(structured(&response)["truncated"], true);
        let after = payload(&response)["next_after_edge"].as_str().unwrap();
        let next = call(
            &mut service,
            tool,
            json!({"revision":revision,"entity":entity,"limit":1,"after_edge":after}),
        );
        assert_ne!(
            payload(&response)["edges"][0]["edge_id"],
            payload(&next)["edges"][0]["edge_id"]
        );
        assert!(payload(&response).to_string().len() < QueryBudget::default().max_response_bytes);
    }
}

#[test]
fn explain_mixed_direction_byte_pages_never_skip_edges() {
    let (mut service, data) = service(ToolProfile::ReadFull, "mixed-page");
    let (_project, root) = cargo_project("mixed-page");
    let scope = seed(&mut service, &root);
    let revision = service
        .engine
        .latest_revision(&ScopeId::new(scope).unwrap())
        .unwrap()
        .unwrap();
    let graph = service.engine.load_revision(&revision).unwrap();
    let target = graph
        .nodes
        .iter()
        .find(|node| node.name == "target")
        .unwrap();
    let entity = format!("resource-{}", target.id);
    let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
    let mut edge: Value = serde_json::from_str(
        &db.query_row(
            "SELECT edge_json FROM relations WHERE source_entity_id=?1 LIMIT 1",
            [&entity],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap();
    db.execute(
        "DELETE FROM relations WHERE snapshot_id=?1",
        [&graph.snapshot.id],
    )
    .unwrap();
    edge["evidence_refs"] = json!([]);
    let expected = vec!["a".to_owned(), "b".to_owned(), "z".to_owned()];
    for (n, id) in expected.iter().enumerate() {
        edge["edge_id"] = json!(id);
        edge["source_entity_id"] = json!(if n == 2 {
            "other".to_owned()
        } else {
            entity.clone()
        });
        edge["target_entity_id"] = json!(if n == 2 {
            entity.clone()
        } else {
            "x".repeat(40_000)
        });
        db.execute("INSERT INTO relations (snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params![graph.snapshot.id,id,edge["source_entity_id"].as_str(),edge["target_entity_id"].as_str(),edge["relation"].as_str(),edge.to_string()]).unwrap();
        db.execute("INSERT INTO relation_run_memberships SELECT ?1,run_id,?2 FROM revision_runs WHERE revision_id=?3 AND role='active'",rusqlite::params![graph.snapshot.id,id,revision]).unwrap();
    }
    let mut seen = Vec::new();
    let mut after = Value::Null;
    for _ in 0..4 {
        let mut args = json!({"revision":revision,"entity":entity,"limit":100});
        if !after.is_null() {
            args["after_edge"] = after;
        }
        let result = call(&mut service, "diskgraph_explain", args);
        let page = payload(&result);
        seen.extend(
            page["edges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["edge_id"].as_str().unwrap().to_owned()),
        );
        after = page["next_after_edge"].clone();
        if page["complete"] == true {
            break;
        }
        assert!(!after.is_null(), "a truncated page must advance");
    }
    assert_eq!(seen, expected);
}

#[test]
fn explain_checks_entity_bytes_before_decoding() {
    let (mut service, data) = service(ToolProfile::ReadFull, "entity-budget");
    let (_project, root) = cargo_project("entity-budget");
    let scope = seed(&mut service, &root);
    let revision = service
        .engine
        .latest_revision(&ScopeId::new(scope).unwrap())
        .unwrap()
        .unwrap();
    let snapshot = service.engine.revision_snapshot(&revision).unwrap();
    let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
    // 超预算且损坏的数据应在 serde 分配之前得到预算错误。
    db.execute(
        "UPDATE entities SET entity_json=?1 WHERE snapshot_id=?2",
        rusqlite::params!["x".repeat(100_000), snapshot.id],
    )
    .unwrap();
    let entity: String = db
        .query_row(
            "SELECT entity_id FROM entities WHERE snapshot_id=?1 LIMIT 1",
            [&snapshot.id],
            |row| row.get(0),
        )
        .unwrap();
    let result = call(
        &mut service,
        "diskgraph_explain",
        json!({"revision":revision,"entity":entity}),
    );
    assert_eq!(
        result["error"]["data"]["business_code"], "budget_exceeded",
        "{result}"
    );
}

#[test]
fn impact_uses_entity_edges_without_decoding_unrelated_relations() {
    let (mut service, data) = service(ToolProfile::ReadFull, "impact-narrow");
    let (project, root) = cargo_project("impact-narrow");
    std::fs::create_dir_all(root.join("other/target")).unwrap();
    std::fs::write(root.join("other/Cargo.toml"), "[package]\nname='other'\n").unwrap();
    std::fs::write(root.join("other/target/bin"), vec![0; 4096]).unwrap();
    let scope = seed(&mut service, &root);
    let revision = service
        .engine()
        .latest_revision(&ScopeId::new(scope.clone()).unwrap())
        .unwrap()
        .unwrap();
    let graph = service.engine().load_revision(&revision).unwrap();
    let target = graph
        .nodes
        .iter()
        .find(|node| node.name == "target")
        .unwrap();
    let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
    assert!(
        db.execute(
            "UPDATE relations SET edge_json = 'invalid JSON' WHERE source_entity_id != ?1",
            [format!("resource-{}", target.id)],
        )
        .unwrap()
            > 0
    );
    let response = call(
        &mut service,
        "diskgraph_impact",
        json!({"scope":scope,"revision":revision,"entity":format!("resource-{}",target.id)}),
    );
    let entries = payload(&response)["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry["relation"] == "rebuildable_by"),
        "{response}"
    );
    assert_eq!(payload(&response)["complete"], true);
    drop(project);
}
