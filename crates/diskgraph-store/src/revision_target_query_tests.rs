//! 目标投影真实原始字节与期限；来源：公开 owned 发布和共享读取账本回归。
use crate::StoreError;
use crate::git_job_test_fixtures::fixture;
use diskgraph_core::{QueryBudget, QueryReadBudget};
use std::time::{Duration, Instant};

fn reads(bytes: usize) -> QueryReadBudget {
    QueryReadBudget::new(
        QueryBudget {
            max_response_bytes: bytes,
            ..QueryBudget::default()
        },
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap()
}

#[test]
fn actual_owner_projection_charges_all_fields_on_the_original_shared_ledger() {
    let (_, store, input, _) = fixture();
    let raw = "snapshot".len() + input.server_id().as_str().len() + input.scope_id().as_str().len();
    let mut reads = reads(raw * 2 - 1);
    let result = store
        .revision_target_with_budget("base", &mut reads)
        .unwrap();
    assert_eq!(
        result,
        (
            "snapshot".into(),
            Some((input.server_id().to_string(), input.scope_id().to_string()))
        )
    );
    assert_eq!(reads.remaining_raw_bytes(), raw - 1);
    assert_eq!(reads.nodes_read(), 0);
    assert!(matches!(
        store.revision_target_with_budget("base", &mut reads),
        Err(StoreError::BudgetExceeded)
    ));
}

#[test]
fn legal_large_snapshot_id_and_expired_deadline_are_refused_without_refreshing_the_budget() {
    let (_, mut store, input, _) = fixture();
    let mut graph = crate::tests::graph(&"x".repeat(2 << 20), 200);
    graph.evidence.clear();
    store.append_staging_nodes("large", &graph.nodes).unwrap();
    store
        .publish_revision_owned(
            "large",
            &graph,
            "large-base",
            200,
            Some((input.server_id().as_str(), input.scope_id().as_str())),
        )
        .unwrap();
    assert!(matches!(
        store.revision_target_with_budget("large-base", &mut reads(4096)),
        Err(StoreError::BudgetExceeded)
    ));
    let mut expired = QueryReadBudget::new(
        QueryBudget::default(),
        Instant::now() - Duration::from_millis(1),
    )
    .unwrap();
    assert!(matches!(
        store.revision_target_with_budget("base", &mut expired),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(expired.nodes_read(), 0);
}

#[test]
fn separated_owner_and_target_share_exact_raw_admission_without_reset() {
    let (_, store, input, _) = fixture();
    let owner_bytes = input.server_id().as_str().len() + input.scope_id().as_str().len();
    let mut ledger = reads(owner_bytes + "snapshot".len());
    assert_eq!(
        store
            .revision_ownership_with_budget("base", &mut ledger)
            .unwrap(),
        Some((input.server_id().to_string(), input.scope_id().to_string()))
    );
    assert_eq!(ledger.remaining_raw_bytes(), "snapshot".len());
    assert_eq!(
        store
            .revision_snapshot_with_budget("base", &mut ledger)
            .unwrap(),
        "snapshot"
    );
    assert_eq!(ledger.remaining_raw_bytes(), 0);
    assert_eq!(ledger.nodes_read(), 0);
    assert!(matches!(
        store.revision_snapshot_with_budget("base", &mut ledger),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(
        ledger.stopped(),
        Some(diskgraph_core::TruncationReason::ByteLimit)
    );

    let mut owner_too_small = reads(owner_bytes - 1);
    assert!(matches!(
        store.revision_ownership_with_budget("base", &mut owner_too_small),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(owner_too_small.remaining_raw_bytes(), owner_bytes - 1);
    assert_eq!(
        owner_too_small.stopped(),
        Some(diskgraph_core::TruncationReason::ByteLimit)
    );
}
