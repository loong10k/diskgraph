use crate::{
    GitEvidenceFailure, GitEvidenceFailureCode, GitEvidenceFailurePhase, GitEvidenceJobInput,
    GitEvidenceLimits, GitEvidenceSummary, JobPublicationReceipt, ScopeId, ServerId,
};

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
fn fixed_input_roundtrip_has_canonical_digest_and_no_client_extensions() {
    let expected = input();
    let mut value = serde_json::to_value(&expected).unwrap();
    assert_eq!(
        serde_json::from_value::<GitEvidenceJobInput>(value.clone()).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::from_value::<GitEvidenceJobInput>(value.clone())
            .unwrap()
            .digest(),
        expected.digest()
    );
    value["authority"] = serde_json::json!({"trusted_local":true});
    assert!(serde_json::from_value::<GitEvidenceJobInput>(value).is_err());
}
#[test]
fn deserialization_revalidates_server_scope_node_revision_and_limits() {
    for (key, bad) in [
        ("server_id", serde_json::json!("../server")),
        ("scope_id", serde_json::json!("scope/escape")),
        ("node_id", serde_json::json!(0)),
        ("node_id", serde_json::json!(u64::MAX)),
        ("base_revision_id", serde_json::json!("../base")),
        ("schema_version", serde_json::json!(0)),
    ] {
        let mut value = serde_json::to_value(input()).unwrap();
        value[key] = bad;
        assert!(
            serde_json::from_value::<GitEvidenceJobInput>(value).is_err(),
            "{key}"
        );
    }
    let mut value = serde_json::to_value(input()).unwrap();
    value["limits"]["max_input_bytes"] = serde_json::json!(u64::MAX);
    assert!(serde_json::from_value::<GitEvidenceJobInput>(value).is_err());
}
#[test]
fn limits_are_finite_and_each_scalar_is_persisted() {
    let limits = GitEvidenceLimits::default();
    assert_eq!(
        (
            limits.max_duration_ms(),
            limits.max_output_bytes(),
            limits.max_input_bytes(),
            limits.max_input_entries(),
            limits.max_capture_bytes(),
            limits.min_free_bytes()
        ),
        (15000, 1 << 20, 64 << 20, 32768, 128 << 20, 64 << 20)
    );
    assert!(GitEvidenceLimits::new(0, 1, 1, 1, 1, 64 << 20).is_err());
    assert!(GitEvidenceLimits::new(1, 1, 1, 1, 1, 0).is_err());
    assert_eq!(
        serde_json::from_str::<GitEvidenceLimits>(&serde_json::to_string(&limits).unwrap())
            .unwrap(),
        limits
    );
}
#[test]
fn unknown_upstream_stays_null_and_safe_summary_does_not_accept_raw_notes() {
    let summary = GitEvidenceSummary::new(2, 1, None, None, 100).unwrap();
    let mut value = serde_json::to_value(&summary).unwrap();
    assert!(value["ahead_of_upstream"].is_null());
    assert!(value["behind_upstream"].is_null());
    assert_eq!(value["local_coverage_complete"], true);
    assert_eq!(value["remote_state_known"], false);
    assert_eq!(
        serde_json::from_value::<GitEvidenceSummary>(value.clone()).unwrap(),
        summary
    );
    value["notes"] = serde_json::json!(["HEAD secret config token"]);
    assert!(serde_json::from_value::<GitEvidenceSummary>(value).is_err());
    assert!(GitEvidenceSummary::new(0, 0, Some(0), None, 100).is_err());
}
#[test]
fn receipt_roundtrip_rejects_digest_or_fence_forgery() {
    let receipt = JobPublicationReceipt::new(
        "job-one".into(),
        input(),
        "snapshot".into(),
        "published".into(),
        "run-one".into(),
        (1, 100),
    )
    .unwrap();
    let original = serde_json::to_value(&receipt).unwrap();
    assert_eq!(
        serde_json::from_value::<JobPublicationReceipt>(original.clone()).unwrap(),
        receipt
    );
    for (key, bad) in [
        ("input_sha256", serde_json::json!("0".repeat(64))),
        ("publishing_fence", serde_json::json!(0)),
        ("schema_version", serde_json::json!(2)),
        ("job_id", serde_json::json!("../job")),
    ] {
        let mut value = original.clone();
        value[key] = bad;
        assert!(
            serde_json::from_value::<JobPublicationReceipt>(value).is_err(),
            "{key}"
        );
    }
}

#[test]
fn observation_fingerprint_binds_safe_observation_and_input_without_claiming_content_hash() {
    let input = input();
    let first = GitEvidenceSummary::new(2, 1, None, None, 100).unwrap();
    let changed = GitEvidenceSummary::new(3, 1, None, None, 100).unwrap();
    assert_ne!(first.observation_fingerprint(&input), input.digest());
    assert_ne!(
        first.observation_fingerprint(&input),
        changed.observation_fingerprint(&input)
    );
    assert_eq!(
        first.observation_fingerprint(&input),
        serde_json::from_str::<GitEvidenceSummary>(&serde_json::to_string(&first).unwrap())
            .unwrap()
            .observation_fingerprint(&input)
    );
    let other = GitEvidenceJobInput::new(
        input.server_id().clone(),
        input.scope_id().clone(),
        "base".into(),
        1,
        input.limits().clone(),
    )
    .unwrap();
    assert_ne!(
        first.observation_fingerprint(&input),
        first.observation_fingerprint(&other)
    );
}

#[test]
fn failure_diagnostic_accepts_only_fixed_typed_codes_without_raw_git_text() {
    let diagnostic = GitEvidenceFailure::new(
        GitEvidenceFailurePhase::Execution,
        GitEvidenceFailureCode::Conflict,
    );
    assert_eq!(
        serde_json::to_value(diagnostic).unwrap(),
        serde_json::json!({"phase":"execution","code":"conflict"})
    );
    assert_eq!(
        serde_json::from_str::<GitEvidenceFailure>(&serde_json::to_string(&diagnostic).unwrap())
            .unwrap(),
        diagnostic
    );
    for value in [
        serde_json::json!({"phase":"execution","code":"fatal: secret path"}),
        serde_json::json!({"phase":"execution","code":"conflict","stderr":"SECRET"}),
    ] {
        assert!(serde_json::from_value::<GitEvidenceFailure>(value).is_err());
    }
}
