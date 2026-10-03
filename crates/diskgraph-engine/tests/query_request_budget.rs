//! D24 已有公开接口的真实行为回归；不依赖尚未新增的 until 签名。

mod query_request_budget {
    pub(super) mod callback_authorizer;
    pub(super) mod fixture;
    mod history_deadline_phases;
}

use diskgraph_core::{BusinessError, QueryBudget};
use diskgraph_engine::EngineError;
use diskgraph_engine::content::{ConservativeProbe, InspectionRequest, InspectionStop};
use diskgraph_store::ControlStore;
use query_request_budget::callback_authorizer::CallbackAuthorizer;
use query_request_budget::fixture::Fixture;
use rusqlite::params;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn is_budget_error(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::Business(BusinessError::BudgetExceeded | BusinessError::Timeout)
            | EngineError::Store(diskgraph_store::StoreError::BudgetExceeded)
    )
}

#[test]
fn initial_tree_authorization_belongs_to_the_default_request_deadline() {
    let fixture = Fixture::new(true);
    let policy = CallbackAuthorizer::new(fixture.policy.clone(), |call| {
        if call == 1 {
            // 真实授权等待耗尽默认整次期限；不模拟 Instant 或放宽生产默认值。
            std::thread::sleep(Duration::from_millis(
                QueryBudget::default().deadline_ms + 100,
            ));
        }
    });
    let started = Instant::now();
    let result = fixture.engine.tree_view(
        &fixture.scope,
        &fixture.revision,
        &fixture.principal,
        &policy,
        2,
        0,
    );
    assert!(started.elapsed() >= Duration::from_millis(1000));
    match result {
        Ok(tree) => assert_eq!(
            tree.root["truncation_reason"], "deadline",
            "late tree was complete"
        ),
        Err(error) => assert!(is_budget_error(&error), "unexpected failure: {error}"),
    }
}

#[test]
fn generic_reader_refuses_scope_revoked_inside_the_final_authorizer() {
    // 无持久策略是可信兼容入口，仍不能把实际撤销的 scope 返回为成功。
    let fixture = Fixture::new(false);
    let path = fixture
        .directory
        .path()
        .join("data/diskgraph-control.sqlite");
    let scope = fixture.scope.clone();
    let policy = CallbackAuthorizer::new(fixture.policy.clone(), move |call| {
        if call == 2 {
            ControlStore::open(&path)
                .unwrap()
                .revoke_scope(&scope)
                .unwrap();
        }
    });
    let result = fixture.engine.with_authorized_revision_reader(
        &fixture.revision,
        &fixture.principal,
        &policy,
        1000,
        |reader, snapshot, _| Ok(reader.node_count(snapshot)?),
    );
    assert!(policy.calls.get() >= 2, "final authorizer was not called");
    assert!(fixture.engine.scope(&fixture.scope).unwrap().revoked);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "the final authorization returned revoked data: {result:?}"
    );
}

#[test]
fn completed_read_does_not_hide_cancellation_behind_the_exhausted_byte_budget() {
    let fixture = Fixture::new(true);
    fixture
        .engine
        .set_content_read(&fixture.scope, &fixture.principal, true)
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let callback_cancel = cancel.clone();
    let policy =
        CallbackAuthorizer::new(fixture.engine.policy_authorizer().unwrap(), move |call| {
            if call == 3 {
                callback_cancel.store(true, Ordering::SeqCst);
            }
        });
    let path = fixture
        .directory
        .path()
        .join("root/a")
        .canonicalize()
        .unwrap();
    let result = fixture
        .engine
        .read_bounded(
            &InspectionRequest {
                scope_id: &fixture.scope,
                principal: &fixture.principal,
                path: &path,
                offset: 0,
                max_bytes: 8,
                cancel: Some(&cancel),
                chunk_bytes: 8,
            },
            &ConservativeProbe,
            &policy,
        )
        .unwrap();
    assert!(cancel.load(Ordering::SeqCst));
    assert_eq!(result.bytes, b"12345678");
    assert_eq!(result.stopped, Some(InspectionStop::Cancelled));
}

#[test]
fn history_header_that_cannot_fit_returns_an_explicit_budget_error() {
    let fixture = Fixture::new(true);
    let result = fixture.engine.compare_revisions_bounded(
        &fixture.revision,
        &fixture.revision,
        0,
        QueryBudget {
            max_response_bytes: 1,
            ..QueryBudget::default()
        },
    );
    match result {
        Err(error) => assert!(is_budget_error(&error), "unexpected failure: {error}"),
        Ok(report) => panic!(
            "one-byte response budget returned {} bytes",
            report.to_json(None).to_string().len()
        ),
    }
}

#[test]
fn escaped_history_root_is_measured_in_the_complete_report() {
    let fixture = Fixture::new(true);
    let locator =
        serde_json::json!({"type":"native_path","value":format!("/{}", "\0".repeat(11_000))})
            .to_string();
    fixture
        .db
        .execute(
            "UPDATE nodes SET locator_key=?1 WHERE snapshot_id=?2 AND parent_id IS NULL",
            params![locator, fixture.snapshot],
        )
        .unwrap();
    let cap = QueryBudget::default().max_response_bytes;
    let result = fixture.engine.compare_revisions_bounded(
        &fixture.revision,
        &fixture.revision,
        0,
        QueryBudget::default(),
    );
    match result {
        Err(error) => assert!(is_budget_error(&error), "unexpected failure: {error}"),
        Ok(report) => assert!(
            report.to_json(None).to_string().len() <= cap,
            "escaped report exceeded {cap} bytes"
        ),
    }
}

#[test]
fn matching_history_nodes_on_both_sides_share_the_node_budget() {
    let fixture = Fixture::new(true);
    let result = fixture.engine.compare_revisions_bounded(
        &fixture.revision,
        &fixture.revision,
        0,
        QueryBudget {
            max_nodes: 1,
            ..QueryBudget::default()
        },
    );
    match result {
        Err(error) => assert!(is_budget_error(&error), "unexpected failure: {error}"),
        Ok(report) => {
            assert_eq!(report.truncated, Some("node_limit"));
            assert!(
                report.rows.is_empty(),
                "one node budget decoded a complete two-sided row"
            );
        }
    }
}

#[test]
fn exhausted_history_budget_does_not_decode_the_next_corrupt_node() {
    let fixture = Fixture::new(true);
    // JSON 与路径排序本身有效，仅真实节点定位类型无效；它必须停留在额度外。
    let changed = fixture.db.execute(
        "UPDATE nodes SET locator_key=json_set(locator_key,'$.type','invalid_fixture_kind') WHERE snapshot_id=?1 AND name='b'",
        [&fixture.snapshot],
    ).unwrap();
    assert_eq!(changed, 1, "exactly one fixture row must be corrupt");
    // 先证明容量内真实格式错误能到达解码器，不以无效 JSON 字段改动充当负控制。
    assert!(
        fixture
            .engine
            .compare_revisions_bounded(
                &fixture.revision,
                &fixture.revision,
                0,
                QueryBudget::default(),
            )
            .is_err()
    );
    let result = fixture.engine.compare_revisions_bounded(
        &fixture.revision,
        &fixture.revision,
        0,
        QueryBudget {
            max_nodes: 1,
            ..QueryBudget::default()
        },
    );
    match result {
        Err(error) => assert!(
            is_budget_error(&error),
            "decoded an out-of-budget node: {error}"
        ),
        Ok(report) => assert_eq!(report.truncated, Some("node_limit")),
    }
}
