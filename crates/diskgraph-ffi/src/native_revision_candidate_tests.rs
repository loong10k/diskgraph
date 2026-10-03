//! 原生候选固定原始 revision，后续发布不得改变已授权请求的批次组合。
use crate::{NativeService, candidates_json, native_reply, scan_native_json};
use diskgraph_core::{QueryBudget, query_deadline};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

fn fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    String,
    String,
    String,
    i64,
) {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("build")).unwrap();
    std::fs::write(root.path().join("build/output"), "abcd").unwrap();
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
    let db = Connection::open(&database).unwrap();
    let revision: String = db
        .query_row(
            "SELECT revision_id FROM graph_revisions WHERE snapshot_id=?1",
            [&snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let node: i64 = db
        .query_row(
            "SELECT id FROM nodes WHERE snapshot_id=?1 AND name='build'",
            [&snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let evidence = json!({"node_id":node,"relation":"rebuildable","subject":"build output","source":"isolated candidate fixture","observed_at_unix_ms":1,"confidence":100});
    db.execute(
        "INSERT INTO evidence VALUES (?1,?2,?3)",
        params![snapshot, node, evidence.to_string()],
    )
    .unwrap();
    (root, data, database, snapshot, revision, node)
}

fn publish_block(database: &str, snapshot: &str, old_revision: &str, node: i64, relation: &str) {
    let db = Connection::open(database).unwrap();
    let target_kind = if relation == "protected_by" {
        "protection_policy"
    } else {
        "process"
    };
    db.execute(
        "INSERT INTO collector_runs VALUES ('ffi-run',?1,'fixture',1,1,1,1,'{}',10)",
        [snapshot],
    )
    .unwrap();
    for (id, kind, identity) in [
        (
            "ffi-resource",
            "resource",
            json!({"node_id":node}).to_string(),
        ),
        ("ffi-target", target_kind, "{}".into()),
    ] {
        let entity = json!({"entity_id":id,"kind":kind,"identity":identity,"display":id,"source_run_id":"ffi-run"});
        db.execute(
            "INSERT INTO entities VALUES (?1,?2,?3,?4)",
            params![snapshot, id, kind, entity.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO entity_run_memberships VALUES (?1,'ffi-run',?2)",
            params![snapshot, id],
        )
        .unwrap();
    }
    let edge = json!({"edge_id":"ffi-block","source_entity_id":"ffi-resource","target_entity_id":"ffi-target","relation":relation,"assertion_kind":"observed","evidence_refs":[["ffi-evidence","supports"]]});
    let evidence = json!({"evidence_id":"ffi-evidence","run_id":"ffi-run","basis":"positive observed blocker","observed_at_unix_ms":1,"expires_at_unix_ms":2,"confidence":100,"input_fingerprint":"fixture"});
    db.execute(
        "INSERT INTO evidence_records VALUES (?1,'ffi-evidence','ffi-run',?2)",
        params![snapshot, evidence.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO relations VALUES (?1,'ffi-block','ffi-resource',?2,'ffi-target',?3)",
        params![snapshot, relation, edge.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO relation_run_memberships VALUES (?1,'ffi-run','ffi-block')",
        [snapshot],
    )
    .unwrap();
    db.execute("INSERT INTO graph_revisions (revision_id,snapshot_id,published_at_unix_ms,writer_generation,locator_writer_generation,native_observation_writer_generation) SELECT 'ffi-new',snapshot_id,published_at_unix_ms+1,10,11,12 FROM graph_revisions WHERE revision_id=?1",[old_revision]).unwrap();
    db.execute("INSERT INTO revision_ownership SELECT 'ffi-new',server_id,scope_id FROM revision_ownership WHERE revision_id=?1",[old_revision]).unwrap();
    db.execute(
        "INSERT INTO revision_runs VALUES ('ffi-new','ffi-run','active')",
        [],
    )
    .unwrap();
    db.execute("UPDATE graph_revisions SET selection_sealed=1,evidence_complete=1 WHERE revision_id='ffi-new'",[]).unwrap();
}

#[test]
fn native_candidate_exports_block_active_typed_process_and_protection() {
    for relation in ["used_by_process", "protected_by"] {
        let (_root, _data, database, snapshot, revision, node) = fixture();
        let service = NativeService::new(database.clone()).unwrap();
        let old: Value =
            serde_json::from_str(&service.candidates_json(snapshot.clone(), 1)).unwrap();
        assert_eq!(old["ok"], true, "{old}");
        assert_eq!(
            old["data"]["candidates"].as_array().unwrap().len(),
            1,
            "{old}"
        );
        let legacy: Value =
            serde_json::from_str(&candidates_json(database.clone(), snapshot.clone(), 1)).unwrap();
        assert_eq!(legacy["data"].as_array().unwrap().len(), 1, "{legacy}");
        publish_block(&database, &snapshot, &revision, node, relation);
        let new: Value =
            serde_json::from_str(&service.candidates_json(snapshot.clone(), 1)).unwrap();
        assert_eq!(new["ok"], true, "{new}");
        assert!(
            new["data"]["candidates"].as_array().unwrap().is_empty(),
            "{new}"
        );
        assert_eq!(new["data"]["review_only"], true);
        assert_eq!(new["data"]["remaining_bytes"], "1");
        let legacy: Value = serde_json::from_str(&candidates_json(database, snapshot, 1)).unwrap();
        assert_eq!(legacy["ok"], true, "{legacy}");
        assert!(legacy["data"].as_array().unwrap().is_empty(), "{legacy}");
    }
}

#[test]
fn native_candidate_callback_keeps_revision_resolved_before_new_publication() {
    let (_root, _data, database, snapshot, revision, node) = fixture();
    let service = NativeService::new(database.clone()).unwrap();
    let answer = native_reply::query_with_revision(
        &service.engine,
        &snapshot,
        query_deadline(QueryBudget::default()).unwrap(),
        Arc::new(AtomicBool::new(false)),
        |reader, selected, deadline| {
            assert_eq!(selected, revision);
            publish_block(&database, &snapshot, &revision, node, "used_by_process");
            assert_eq!(
                reader.revision_for_snapshot(&snapshot).unwrap().as_deref(),
                Some("ffi-new")
            );
            let result = reader
                .candidate_selection_for_revision_until(
                    selected,
                    1,
                    QueryBudget::default(),
                    deadline,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({"count":result.candidates.len(),"revision":selected}))
        },
        || {},
    )
    .unwrap();
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["data"]["count"], 1, "{answer}");
    assert_eq!(answer["data"]["revision"], revision);
}
