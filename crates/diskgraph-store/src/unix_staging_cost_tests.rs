//! Unix实际编码计费的隔离回归。
use crate::staging_unix_observation_encoded_cost;
use diskgraph_core::{UnixFileObservation, UnixObservationGap};
#[test]
fn measured_payload_matches_the_actual_staging_row() {
    let (_, mut store, _, _) = crate::process_job_test_fixtures::fixture();
    let mut graph = crate::tests::graph("encoded-cost", 200);
    graph.nodes[1].kind = diskgraph_core::NodeKind::File;
    graph.nodes[1].directories = 0;
    let locator = diskgraph_core::QualifiedLocator::from_parts(
        diskgraph_core::LocatorKind::NativePath,
        diskgraph_core::LocatorEncoding::UnixBytes,
        b"/tmp/diskgraph-test/cache".to_vec(),
        "/tmp/diskgraph-test/cache".into(),
    )
    .unwrap();
    let observation = UnixFileObservation::new(
        crate::process_job_test_fixtures::epoch(),
        0o100600,
        100,
        (1, 2),
        (3, 4),
        (10, 11),
    )
    .unwrap();
    for (index, (value, gap)) in [
        (Some(&observation), None),
        (None, Some(UnixObservationGap::Denied)),
    ]
    .into_iter()
    .enumerate()
    {
        let job = format!("encoded-cost-{index}");
        store
            .append_staging_located_iter(
                &job,
                std::iter::once((&graph.nodes[1], &locator, Some(1))),
            )
            .unwrap();
        store
            .append_staging_unix_observations_checked(
                &job,
                std::iter::once((2, value, gap)),
                || Ok(()),
            )
            .unwrap();
        // 从实际SQL行读取载荷，而非再调用编码器作为预期，防止计量和写入偏离。
        let (raw, code, generation): (Option<Vec<u8>>, Option<String>, i64) = store.connection.query_row(
            "SELECT observation_raw,gap,writer_generation FROM scan_staging_unix_observations WHERE job_id=?1 AND node_id=2",
            [&job], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(generation, 14);
        let payload_len = raw
            .as_ref()
            .map_or_else(|| code.as_ref().unwrap().len(), Vec::len);
        assert_eq!(
            staging_unix_observation_encoded_cost(value, gap).unwrap(),
            payload_len as u64 + 16
        );
        assert_eq!(raw, value.map(|v| v.encode().unwrap()));
        assert_eq!(code.as_deref(), gap.map(|v| v.code()));
    }
}
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
