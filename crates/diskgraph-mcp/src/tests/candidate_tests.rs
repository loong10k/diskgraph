//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use super::support::{call, cargo_project, payload, seed, service};
use crate::protocol::ToolProfile;
use diskgraph_core::ScopeId;
use serde_json::json;

#[test]
fn zero_target_candidates_do_not_decode_the_full_revision() {
    let (mut service, data) = service(ToolProfile::ReadFull, "zero-candidates");
    let (project, root) = cargo_project("zero-candidates");
    let scope = seed(&mut service, &root);
    let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute(
        "UPDATE nodes SET kind = 'invalid-kind' WHERE name = 'bin'",
        [],
    )
    .unwrap();
    let response = call(
        &mut service,
        "diskgraph_candidates",
        json!({"scope":scope,"target_bytes":0}),
    );
    assert_eq!(payload(&response)["candidates"], json!([]));
    drop(project);
}

#[test]
fn positive_target_candidates_use_a_narrow_read_and_report_the_target_gap() {
    let (mut service, data) = service(ToolProfile::ReadFull, "positive-candidates");
    let (project, root) = cargo_project("positive-candidates");
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
    db.execute(
        "INSERT INTO evidence (snapshot_id, node_id, evidence_json) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            graph.snapshot.id,
            i64::try_from(target.id).unwrap(),
            json!({
                "node_id": target.id, "relation": "rebuildable", "subject": "fixture",
                "source": "test", "observed_at_unix_ms": 1, "confidence": 100,
            })
            .to_string()
        ],
    )
    .unwrap();
    db.execute(
        "UPDATE nodes SET kind = 'invalid-kind' WHERE name = 'bin'",
        [],
    )
    .unwrap();
    let response = call(
        &mut service,
        "diskgraph_candidates",
        json!({"scope":scope,"target_bytes":u64::MAX}),
    );
    assert_eq!(payload(&response)["review_only"], true);
    assert!(
        payload(&response)["candidates"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert_eq!(payload(&response)["complete"], true);
    assert!(
        payload(&response)["remaining_bytes"]
            .as_str()
            .and_then(|bytes| bytes.parse::<u64>().ok())
            .is_some_and(|bytes| bytes > 0)
    );
    drop(project);
}

#[test]
fn incomplete_candidates_do_not_decode_the_full_revision() {
    let (mut service, data) = service(ToolProfile::ReadFull, "incomplete-candidates");
    let (project, root) = cargo_project("incomplete-candidates");
    let scope = seed(&mut service, &root);
    let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute(
        "UPDATE snapshots SET snapshot_json = json_set(snapshot_json, '$.coverage.complete', json('false'))",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE nodes SET kind = 'invalid-kind' WHERE name = 'bin'",
        [],
    )
    .unwrap();
    let response = call(
        &mut service,
        "diskgraph_candidates",
        json!({"scope":scope,"target_bytes":1}),
    );
    assert_eq!(payload(&response)["candidates"], json!([]));
    drop(project);
}
