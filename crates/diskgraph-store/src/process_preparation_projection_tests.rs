//! 真实 SQLite 借用投影与同账本测试；不证明 native/provider 能力或 RSS。
use crate::process_job_test_fixtures::fixture;
use crate::{Result, StoreError};
use diskgraph_core::{Locator, ProcessEvidenceLimits, QueryBudget, QueryReadBudget};
use std::time::{Duration, Instant};

fn budget_error(_: u64, _: u64, _: u64) -> Result<()> {
    Err(StoreError::BudgetExceeded)
}

#[test]
fn process_preparation_projection_shares_control_costs_and_preserves_original_values() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let (limits, raw, allocation) = control.process_job_limits_with_cost(&job.job_id).unwrap();
    assert_eq!(&limits, input.limits());
    assert!(raw > 0 && allocation > 0);
    let scope = control.scope(input.scope_id()).unwrap();
    let mut totals = (raw, 1_u64, allocation);
    let mut admit = |raw: u64, entries: u64, allocation: u64| {
        totals.0 += raw;
        totals.1 += entries;
        totals.2 += allocation;
        Ok(())
    };
    assert_eq!(
        control
            .process_evidence_job_input_with_admission(&job.job_id, &mut admit)
            .unwrap(),
        input
    );
    assert_eq!(
        control
            .job_request_authority_with_admission(&job.job_id, &mut admit)
            .unwrap(),
        Some(authority)
    );
    assert_eq!(
        control
            .scope_with_admission(input.scope_id(), &mut admit)
            .unwrap(),
        scope
    );
    assert_eq!(
        control
            .existing_server_id_with_admission(&mut admit)
            .unwrap(),
        *input.server_id()
    );
    assert!(totals.0 > raw && totals.1 == 5 && totals.2 > allocation);
    // 第二次真实读取仍消费同一剩余额度，不能因另一个投影构建新预算。
    let mut remaining = totals.0 - raw;
    let mut shared = |raw: u64, _: u64, _: u64| {
        remaining = remaining
            .checked_sub(raw)
            .ok_or(StoreError::BudgetExceeded)?;
        Ok(())
    };
    control
        .process_evidence_job_input_with_admission(&job.job_id, &mut shared)
        .unwrap();
    control
        .job_request_authority_with_admission(&job.job_id, &mut shared)
        .unwrap();
    control
        .scope_with_admission(input.scope_id(), &mut shared)
        .unwrap();
    control
        .existing_server_id_with_admission(&mut shared)
        .unwrap();
    assert!(matches!(
        control.process_evidence_job_input_with_admission(&job.job_id, &mut shared),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(remaining, 0);
}

#[test]
fn process_preparation_projection_preserves_graph_target_and_missing_gap_semantics() {
    let (_, graph, input, _) = fixture();
    let mut old = QueryReadBudget::new(
        QueryBudget::default(),
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    let mut totals = (0_u64, 0_u64, 0_u64);
    let mut admit = |raw, entries, allocation| {
        totals.0 += raw;
        totals.1 += entries;
        totals.2 += allocation;
        Ok(())
    };
    assert_eq!(
        graph
            .revision_ownership_with_admission(input.base_revision_id(), &mut admit)
            .unwrap(),
        graph
            .revision_ownership_with_budget(input.base_revision_id(), &mut old)
            .unwrap()
    );
    assert_eq!(
        graph
            .revision_snapshot_with_admission(input.base_revision_id(), &mut admit)
            .unwrap(),
        "process-snapshot"
    );
    assert_eq!(
        graph
            .node_with_admission("process-snapshot", 2, &mut admit)
            .unwrap(),
        graph.node("process-snapshot", 2).unwrap()
    );
    assert_eq!(
        graph
            .unix_observation_with_admission("process-snapshot", 2, &mut admit)
            .unwrap(),
        graph
            .unix_observation_bounded("process-snapshot", 2, &mut old)
            .unwrap()
    );
    assert_eq!(
        graph
            .unix_observation_with_admission("process-snapshot", 1, &mut admit)
            .unwrap(),
        graph
            .unix_observation_bounded("process-snapshot", 1, &mut old)
            .unwrap()
    );
    #[cfg(unix)]
    assert_eq!(
        graph
            .native_locator_with_admission("process-snapshot", 2, &mut admit)
            .unwrap(),
        graph
            .native_locator_bounded("process-snapshot", 2, &mut old)
            .unwrap()
    );
    #[cfg(not(unix))]
    assert!(matches!(
        graph.native_locator_with_admission("process-snapshot", 2, &mut admit),
        Err(StoreError::UnsupportedLocator(_))
    ));
    assert!(
        graph
            .node_with_admission("process-snapshot", 999, &mut admit)
            .unwrap()
            .is_none()
    );
    assert!(totals.0 > 0 && totals.1 >= 6 && totals.2 > totals.0);
}

#[test]
fn process_preparation_projection_rejects_before_decoding_invalid_borrowed_node() {
    let (_, graph, _, _) = fixture();
    // SQLite 表达式索引必须保持可读：合法 JSON 缺少 DiskNode 必需字段，只有 typed 解码拒绝。
    graph.connection.execute("UPDATE nodes SET kind=NULL,node_json='{}' WHERE snapshot_id='process-snapshot' AND id=2", []).unwrap();
    let mut reached = false;
    let mut admit = |raw, _: u64, _: u64| {
        if raw > 0 {
            reached = true;
            return Err(StoreError::BudgetExceeded);
        }
        Ok(())
    };
    assert!(matches!(
        graph.node_with_admission("process-snapshot", 2, &mut admit),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(reached, "raw admission was not reached before the decoder");
    assert!(matches!(
        graph.node_with_admission("process-snapshot", 2, &mut |_, _, _| Ok(())),
        Err(StoreError::Json(_))
    ));
}

#[test]
fn process_preparation_projection_checks_every_control_and_graph_entry() {
    let (mut control, graph, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert!(matches!(
        control.process_evidence_job_input_with_admission(&job.job_id, &mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        control.job_request_authority_with_admission(&job.job_id, &mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        control.scope_with_admission(input.scope_id(), &mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        control.existing_server_id_with_admission(&mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        graph.revision_ownership_with_admission(input.base_revision_id(), &mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        graph.revision_snapshot_with_admission(input.base_revision_id(), &mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        graph.node_with_admission("process-snapshot", 2, &mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        graph.native_locator_with_admission("process-snapshot", 2, &mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        graph.unix_observation_with_admission("process-snapshot", 2, &mut budget_error),
        Err(StoreError::BudgetExceeded)
    ));
}

#[test]
fn process_preparation_projection_rejects_large_legal_scope_before_rust_owning() {
    use crate::git_raw_allocation_tests::{isolated, measure};
    let name = "process_preparation_projection_tests::process_preparation_projection_rejects_large_legal_scope_before_rust_owning";
    if isolated(name) {
        return;
    }
    let (mut control, _, _, _) = fixture();
    let locator = Locator::from_document_uri(format!("test://{}", "x".repeat(2 << 20)));
    let id = control
        .register_scope(&locator, Some(&"v".repeat(2 << 20)))
        .unwrap();
    assert_eq!(control.scope(&id).unwrap().root, locator);
    let mut reached = false;
    let mut admit = |raw, entries, allocation| {
        if raw > 0 {
            reached = true;
            assert!(raw > 4 << 20 && allocation > raw && entries == 1);
            return Err(StoreError::BudgetExceeded);
        }
        Ok(())
    };
    let (result, bytes) = measure(|| control.scope_with_admission(&id, &mut admit));
    assert!(matches!(result, Err(StoreError::BudgetExceeded)));
    assert!(reached);
    eprintln!("borrowed scope raw >4MiB, Rust requested={bytes}");
    assert!(
        bytes < 65536,
        "large scope was owned before callback: {bytes}"
    );
}

#[test]
fn process_preparation_projection_bootstrap_is_not_full_input_authorization() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    control
        .connection
        .execute_batch("DROP TRIGGER process_job_input_immutable_update;")
        .unwrap();
    let mut raw = serde_json::to_value(&input).unwrap();
    raw["indexed_epoch"] = serde_json::json!({"forged":"invalid"});
    control
        .connection
        .execute(
            "UPDATE process_evidence_job_inputs SET input_json=?1",
            [raw.to_string()],
        )
        .unwrap();
    assert_eq!(
        control.process_job_limits_with_cost(&job.job_id).unwrap().0,
        ProcessEvidenceLimits::default()
    );
    assert!(matches!(
        control.process_evidence_job_input_with_admission(&job.job_id, &mut |_, _, _| Ok(())),
        Err(StoreError::InvalidGraph(_))
    ));
    let raw = raw
        .to_string()
        .replace("\"limits\":{", "\"limits\":{\"max_duration_ms\":1,");
    control
        .connection
        .execute(
            "UPDATE process_evidence_job_inputs SET input_json=?1",
            [raw],
        )
        .unwrap();
    assert!(matches!(
        control.process_job_limits_with_cost(&job.job_id),
        Err(StoreError::InvalidGraph(_))
    ));
}
