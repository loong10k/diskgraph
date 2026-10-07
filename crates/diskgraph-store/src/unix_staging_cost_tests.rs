//! Unix实际编码计费的隔离回归。
use crate::staging_unix_observation_encoded_cost;
use diskgraph_core::{UnixFileObservation, UnixObservationGap};
#[test]
fn actual_cost_accepts_equal_budget_and_rejects_one_byte_less() {
    use diskgraph_core::{BudgetDecision, BudgetUsage, ScanBudget, ScanBudgetStop};
    let cost =
        staging_unix_observation_encoded_cost(None, Some(UnixObservationGap::Denied)).unwrap();
    let budget = ScanBudget {
        max_staging_bytes: cost,
        ..ScanBudget::default()
    };
    assert_eq!(
        budget.charge_node(&mut BudgetUsage::default(), cost),
        BudgetDecision::Continue
    );
    let smaller = ScanBudget {
        max_staging_bytes: cost - 1,
        ..budget
    };
    assert_eq!(
        smaller.charge_node(&mut BudgetUsage::default(), cost),
        BudgetDecision::Stop(ScanBudgetStop::StagingLimit)
    );
}
#[test]
fn every_gap_charges_its_exact_code_and_two_integer_fields() {
    for gap in [
        UnixObservationGap::NotCaptured,
        UnixObservationGap::Unsupported,
        UnixObservationGap::Denied,
        UnixObservationGap::Changed,
        UnixObservationGap::CaptureFailed,
        UnixObservationGap::TreeMismatch,
    ] {
        assert_eq!(
            staging_unix_observation_encoded_cost(None, Some(gap)).unwrap(),
            gap.code().len() as u64 + 16
        );
    }
}
#[test]
fn complete_observation_charges_real_validated_encoding() {
    let observation = UnixFileObservation::new(
        crate::process_job_test_fixtures::epoch(),
        0o100600,
        100,
        (1, 2),
        (3, 4),
        (10, 11),
    )
    .unwrap();
    assert_eq!(
        staging_unix_observation_encoded_cost(Some(&observation), None).unwrap(),
        observation.encode().unwrap().len() as u64 + 16
    );
}
#[test]
fn missing_or_conflicting_payload_is_rejected() {
    assert!(staging_unix_observation_encoded_cost(None, None).is_err());
    let observation = UnixFileObservation::new(
        crate::process_job_test_fixtures::epoch(),
        0o100600,
        100,
        (1, 2),
        (3, 4),
        (10, 11),
    )
    .unwrap();
    assert!(
        staging_unix_observation_encoded_cost(Some(&observation), Some(UnixObservationGap::Denied))
            .is_err()
    );
}
