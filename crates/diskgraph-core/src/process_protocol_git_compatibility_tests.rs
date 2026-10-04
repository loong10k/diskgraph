//! D42 独立 process 协议的旧 Git v1 字节兼容正控；来源：原生 Rust EC-02 / EV-05。
use crate::{GitEvidenceJobInput, GitEvidenceLimits, JobPublicationReceipt, ScopeId, ServerId};

const GIT_INPUT: &str = r#"{"schema_version":1,"server_id":"server","scope_id":"scope","base_revision_id":"base","node_id":2,"limits":{"max_duration_ms":15000,"max_output_bytes":1048576,"max_input_bytes":67108864,"max_input_entries":32768,"max_capture_bytes":134217728,"min_free_bytes":67108864}}"#;
const INPUT_SHA256: &str = "2e6afc75bba5d05a3231b4372850b217f38985b5c8c1f93f2eefebe889a9aad9";
const GIT_RECEIPT: &str = r#"{"schema_version":1,"job_id":"job-one","input_sha256":"2e6afc75bba5d05a3231b4372850b217f38985b5c8c1f93f2eefebe889a9aad9","input":{"schema_version":1,"server_id":"server","scope_id":"scope","base_revision_id":"base","node_id":2,"limits":{"max_duration_ms":15000,"max_output_bytes":1048576,"max_input_bytes":67108864,"max_input_entries":32768,"max_capture_bytes":134217728,"min_free_bytes":67108864}},"snapshot_id":"snapshot","revision_id":"published","run_id":"run-one","publishing_fence":1,"committed_at_unix_ms":100}"#;

fn input() -> GitEvidenceJobInput {
    GitEvidenceJobInput::new(
        ServerId::new("server").unwrap(),
        ScopeId::new("scope").unwrap(),
        "base".into(),
        2,
        GitEvidenceLimits::default(),
    )
    .unwrap()
}

#[test]
fn git_v1_input_bytes_and_digest_are_preserved_exactly() {
    let original = input();
    assert_eq!(serde_json::to_string(&original).unwrap(), GIT_INPUT);
    assert_eq!(original.digest(), INPUT_SHA256);
    assert_eq!(
        serde_json::from_str::<GitEvidenceJobInput>(GIT_INPUT).unwrap(),
        original
    );
}

#[test]
fn git_v1_receipt_keeps_its_concrete_input_and_exact_bytes() {
    let original = JobPublicationReceipt::new(
        "job-one".into(),
        input(),
        "snapshot".into(),
        "published".into(),
        "run-one".into(),
        (1, 100),
    )
    .unwrap();
    // 编译期同时保留旧 getter 的具体 Git 返回类型，不把旧 API 改成通用 enum。
    let old_input: &GitEvidenceJobInput = original.input();
    assert_eq!(old_input, &input());
    assert_eq!(serde_json::to_string(&original).unwrap(), GIT_RECEIPT);
    assert_eq!(
        serde_json::from_str::<JobPublicationReceipt>(GIT_RECEIPT).unwrap(),
        original
    );
}

#[test]
fn git_v1_decoders_do_not_silently_drop_process_fields() {
    let mut new_input: serde_json::Value = serde_json::from_str(GIT_INPUT).unwrap();
    new_input["method"] = serde_json::json!("linux_procfs_v1");
    new_input["indexed_epoch"] = serde_json::json!({"inode": 1});
    assert!(serde_json::from_value::<GitEvidenceJobInput>(new_input).is_err());
    let mut new_receipt: serde_json::Value = serde_json::from_str(GIT_RECEIPT).unwrap();
    new_receipt["kind"] = serde_json::json!("process_evidence");
    assert!(serde_json::from_value::<JobPublicationReceipt>(new_receipt).is_err());
}
