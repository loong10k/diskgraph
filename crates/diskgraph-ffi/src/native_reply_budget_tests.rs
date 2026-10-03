//! 原生会话真实 reply 生命周期的到期、撤权回归，不修改 UniFFI 导出。

use super::NativeService;
use crate::{local_principal, scan_native_json};
use diskgraph_core::QueryBudget;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

fn session() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<NativeService>,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_string_lossy()
        .into_owned();
    let scan: Value = serde_json::from_str(&scan_native_json(
        database.clone(),
        root.path().to_string_lossy().into_owned(),
    ))
    .unwrap();
    assert_eq!(scan["ok"], true, "{scan}");
    let snapshot = scan["data"]["snapshot_id"].as_str().unwrap().to_owned();
    let service = NativeService::new(database).unwrap();
    (root, data, service, snapshot)
}

#[test]
fn reply_wait_cannot_return_an_expired_native_success() {
    let (_root, _data, service, snapshot) = session();
    let answer = service.query_then(
        &snapshot,
        |_| Ok(json!("late native data")),
        || {
            std::thread::sleep(Duration::from_millis(
                QueryBudget::default().deadline_ms + 100,
            ));
        },
    );
    assert!(
        answer.is_err(),
        "expired native request returned success: {answer:?}"
    );
}

#[test]
fn scope_revoked_at_reply_boundary_cannot_escape_the_session() {
    let (_root, _data, service, snapshot) = session();
    let principal = local_principal().unwrap();
    let policy = service.engine.policy_authorizer().unwrap();
    let scopes = service.engine.list_scopes(&principal, &policy).unwrap();
    assert_eq!(scopes.len(), 1);
    let scope = scopes[0].scope_id.clone();
    let answer = service.query_then(
        &snapshot,
        |_| Ok(json!("revoked native data")),
        || {
            service
                .engine
                .revoke_scope(
                    &scope,
                    &principal,
                    &service.engine.policy_authorizer().unwrap(),
                )
                .unwrap();
        },
    );
    assert!(
        answer.is_err(),
        "revoked native request returned data: {answer:?}"
    );
}

#[test]
fn native_envelope_accepts_the_exact_byte_cap_and_refuses_one_more() {
    let (_root, _data, service, snapshot) = session();
    let cap = QueryBudget::default().max_response_bytes;
    let framing = json!({"schema_version":1,"ok":true,"data":""})
        .to_string()
        .len();
    let exact = service
        .query(&snapshot, |_| Ok(json!("x".repeat(cap - framing))))
        .unwrap();
    assert_eq!(exact.len(), cap);
    let over = service.query(&snapshot, |_| Ok(json!("x".repeat(cap - framing + 1))));
    assert!(over.unwrap_err().contains("budget_exceeded"));
}

#[test]
fn native_partial_deadline_preserves_the_selected_prefix_and_gap() {
    let (_root, _data, service, snapshot) = session();
    let text = service
        .query_then(
            &snapshot,
            |_| {
                Ok(json!({
                    "candidates":[], "complete":true, "truncated":null,
                    "selected_bytes":"8", "remaining_bytes":"12", "review_only":true
                }))
            },
            || {
                std::thread::sleep(Duration::from_millis(
                    QueryBudget::default().deadline_ms + 100,
                ))
            },
        )
        .unwrap();
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["data"]["complete"], false);
    assert_eq!(reply["data"]["truncated"], "deadline");
    assert_eq!(reply["data"]["selected_bytes"], "8");
    assert_eq!(reply["data"]["remaining_bytes"], "12");
    assert!(text.len() <= QueryBudget::default().max_response_bytes);
}

#[test]
fn legacy_explain_rejects_raw_evidence_before_invalid_confidence_decode() {
    let (_root, data, _service, snapshot) = session();
    let database = data.path().join("graph.sqlite");
    let connection = rusqlite::Connection::open(&database).unwrap();
    let evidence = json!({"node_id":1,"relation":"rebuildable_by", "subject":"x".repeat(100_000),
        "source":"isolated raw-budget fixture","observed_at_unix_ms":1,"confidence":300});
    connection
        .execute(
            "INSERT INTO evidence(snapshot_id,node_id,evidence_json) VALUES(?1,1,?2)",
            rusqlite::params![snapshot, evidence.to_string()],
        )
        .unwrap();
    drop(connection);
    let reply: Value = serde_json::from_str(&crate::explain_json(
        database.to_string_lossy().into_owned(),
        snapshot,
        1,
    ))
    .unwrap();
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply["error"].as_str().unwrap().contains("budget"),
        "{reply}"
    );
    assert!(reply.get("data").is_none());
}

#[test]
fn native_node_rejects_an_oversized_borrowed_name_before_owned_decode() {
    let (_root, data, service, snapshot) = session();
    let connection = rusqlite::Connection::open(data.path().join("graph.sqlite")).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE nodes SET name=?1 WHERE snapshot_id=?2 AND id=1",
                rusqlite::params!["x".repeat(100_000), snapshot]
            )
            .unwrap(),
        1
    );
    drop(connection);
    let reply: Value = serde_json::from_str(&service.node_json(snapshot, 1)).unwrap();
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply["error"].as_str().unwrap().contains("budget"),
        "{reply}"
    );
    assert!(reply.get("data").is_none());
}

#[test]
fn native_error_diagnostic_is_bounded_after_json_escaping() {
    let (_root, data, service, snapshot) = session();
    let connection = rusqlite::Connection::open(data.path().join("graph.sqlite")).unwrap();
    assert_eq!(
        connection
            .execute(
                "UPDATE nodes SET kind=?1 WHERE snapshot_id=?2 AND id=1",
                rusqlite::params!["\0".repeat(11_000), snapshot]
            )
            .unwrap(),
        1
    );
    drop(connection);
    let text = service.node_json(snapshot, 1);
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(reply.get("data").is_none());
    assert!(
        text.len() <= QueryBudget::default().max_response_bytes,
        "native failure diagnostic has {} encoded bytes",
        text.len()
    );
}
