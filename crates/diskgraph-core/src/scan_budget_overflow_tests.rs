//! 扫描预算极值计数回归。
use crate::{BudgetDecision, BudgetUsage, ScanBudget, ScanBudgetStop};
fn unbounded_ceiling() -> ScanBudget {
    ScanBudget {
        max_nodes: u64::MAX,
        max_staging_bytes: u64::MAX,
        ..ScanBudget::default()
    }
}
#[test]
fn node_overflow_stops_before_byte_accounting() {
    let mut usage = BudgetUsage {
        nodes: u64::MAX,
        staged_bytes: 7,
        ..BudgetUsage::default()
    };
    assert_eq!(
        unbounded_ceiling().charge_node(&mut usage, 1),
        BudgetDecision::Stop(ScanBudgetStop::NodeLimit)
    );
    assert_eq!(usage.nodes, u64::MAX);
    assert_eq!(usage.staged_bytes, 7);
}
#[test]
fn encoded_byte_overflow_is_not_an_equal_ceiling() {
    let mut usage = BudgetUsage {
        staged_bytes: u64::MAX,
        ..BudgetUsage::default()
    };
    assert_eq!(
        unbounded_ceiling().charge_node(&mut usage, 1),
        BudgetDecision::Stop(ScanBudgetStop::StagingLimit)
    );
    assert_eq!(usage.staged_bytes, u64::MAX);
}
#[test]
fn exact_representable_ceiling_is_still_allowed() {
    let mut usage = BudgetUsage {
        nodes: u64::MAX - 1,
        staged_bytes: u64::MAX - 1,
        ..BudgetUsage::default()
    };
    assert_eq!(
        unbounded_ceiling().charge_node(&mut usage, 1),
        BudgetDecision::Continue
    );
    assert_eq!(usage.nodes, u64::MAX);
    assert_eq!(usage.staged_bytes, u64::MAX);
}
