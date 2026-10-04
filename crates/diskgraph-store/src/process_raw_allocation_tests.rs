//! Process 原始列准入的真实 Rust requested 观测；不等于 SQLite C、I/O 或 RSS。
use crate::StoreError;
use crate::git_raw_allocation_tests::{isolated, measure};
use crate::process_job_test_fixtures::{batch, fixture};
#[test]
fn oversized_input_server_is_rejected_before_owning_raw_field() {
    if isolated(
        "process_raw_allocation_tests::oversized_input_server_is_rejected_before_owning_raw_field",
    ) {
        return;
    }
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert_eq!(
        control.process_evidence_job_input(&job.job_id).unwrap(),
        input
    );
    control
        .connection
        .execute("UPDATE server SET server_id=?1", ["x".repeat(2 << 20)])
        .unwrap();
    let (result, bytes) = measure(|| control.process_evidence_job_input(&job.job_id));
    assert!(matches!(result, Err(StoreError::InvalidGraph(_))));
    eprintln!("process input redundant 2MiB server; Rust requested={bytes}");
    assert!(bytes < 65536, "raw server owned before admission: {bytes}");
}
#[test]
fn oversized_receipt_server_is_rejected_before_owning_raw_field() {
    if isolated(
        "process_raw_allocation_tests::oversized_receipt_server_is_rejected_before_owning_raw_field",
    ) {
        return;
    }
    let (_, mut graph, input, _) = fixture();
    let receipt = graph
        .publish_process_collector_revision_checked(
            "job",
            &input,
            (1, 1001),
            "published",
            &batch(&input, "published", true, false),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        graph.process_job_publication_receipt("job").unwrap(),
        Some(receipt)
    );
    graph
        .connection
        .execute_batch("DROP TRIGGER process_receipt_no_update;")
        .unwrap();
    graph
        .connection
        .execute(
            "UPDATE process_job_publication_receipts SET server_id=?1",
            ["x".repeat(2 << 20)],
        )
        .unwrap();
    let (result, bytes) = measure(|| graph.process_job_publication_receipt("job"));
    assert!(matches!(result, Err(StoreError::InvalidGraph(_))));
    eprintln!("process receipt redundant 2MiB server; Rust requested={bytes}");
    assert!(bytes < 65536, "raw server owned before admission: {bytes}");
}
