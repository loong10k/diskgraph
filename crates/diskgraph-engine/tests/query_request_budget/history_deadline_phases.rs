//! 分别固定准备前与真实报告生成后的到期，避免把宿主调度速度写成返回契约。

use super::callback_authorizer::CallbackAuthorizer;
use super::fixture::Fixture;
use diskgraph_core::QueryBudget;
use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use std::time::{Duration, Instant};

#[test]
fn history_expired_before_preparation_returns_no_report() {
    let f = Fixture::new(true);
    let result = f.engine.compare_revisions_bounded_until(
        &f.revision,
        &f.revision,
        0,
        QueryBudget::default(),
        Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Store(StoreError::BudgetExceeded))
    ));
}

#[test]
fn history_expiring_after_read_preserves_rows_and_partial_statistics() {
    let f = Fixture::new(true);
    let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
    let policy = CallbackAuthorizer::new(f.policy.clone(), move |call| {
        // 两侧初始授权之后的第一次回调发生在真实报告读取完成之后。
        if call == 3 {
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            );
        }
    });
    let report = f
        .engine
        .compare_revisions_until(
            &f.revision,
            &f.revision,
            0,
            QueryBudget::default(),
            &f.principal,
            &policy,
            deadline,
        )
        .unwrap();
    assert!(policy.calls.get() >= 4);
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.summary.same, 2);
    let value = report.to_json(None);
    assert_eq!(value["complete"], false);
    assert_eq!(value["truncation_reason"], "deadline");
    assert_eq!(value["summary_is_partial"], true);
}
