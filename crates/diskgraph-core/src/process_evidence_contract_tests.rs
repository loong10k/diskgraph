//! D42 固定协议的纯值验收，不替代原生身份来源验收。
use crate::{
    GitEvidenceJobInput, IndexedFileEpoch, ProcessEvidenceJobInput, ProcessEvidenceLimits,
    ProcessEvidenceSummary, ProcessJobPublicationReceipt, ProcessObservationCode,
    ProcessObservationCoverage, ProcessObservationMethod, ProcessStartupIdentity, ScopeId,
    ServerId, UnixFileObservation,
};
fn epoch() -> IndexedFileEpoch {
    IndexedFileEpoch::LinuxHandle {
        device: 1,
        inode: 2,
        filesystem_domain_sha256: [3; 32],
        handle_type: 1,
        handle_bytes: vec![4; 12],
    }
}
fn input() -> ProcessEvidenceJobInput {
    ProcessEvidenceJobInput::new(
        ServerId::new("server").unwrap(),
        ScopeId::new("scope").unwrap(),
        "base".into(),
        2,
        ProcessObservationMethod::LinuxProcfsV1,
        epoch(),
        ProcessEvidenceLimits::default(),
    )
    .unwrap()
}
#[test]
fn independent_process_v1_roundtrips_without_becoming_git() {
    let original = input();
    let raw = serde_json::to_vec(&original).unwrap();
    assert_eq!(
        serde_json::from_slice::<ProcessEvidenceJobInput>(&raw).unwrap(),
        original
    );
    assert!(serde_json::from_slice::<GitEvidenceJobInput>(&raw).is_err());
    let mut v = serde_json::to_value(&original).unwrap();
    v["path"] = serde_json::json!("/forged");
    assert!(serde_json::from_value::<ProcessEvidenceJobInput>(v).is_err());
}
#[test]
fn zero_or_legacy_epoch_and_wrong_platform_are_rejected() {
    assert!(
        IndexedFileEpoch::MacGeneration {
            device: 1,
            inode: 2,
            filesystem_domain_sha256: [3; 32],
            generation: 0,
            birth_seconds: 1,
            birth_nanos: 0
        }
        .validate()
        .is_err()
    );
    assert!(
        ProcessEvidenceJobInput::new(
            ServerId::new("server").unwrap(),
            ScopeId::new("scope").unwrap(),
            "base".into(),
            2,
            ProcessObservationMethod::MacLibprocV1,
            epoch(),
            ProcessEvidenceLimits::default()
        )
        .is_err()
    );
    let v = serde_json::json!({"kind":"linux_handle","device":1,"inode":2,"filesystem_domain_sha256":vec![3;32],"handle_type":1,"handle_bytes":[]});
    assert!(serde_json::from_value::<IndexedFileEpoch>(v).is_err());
}
#[test]
fn full_windows_volume_and_128bit_epoch_do_not_truncate() {
    let epoch = IndexedFileEpoch::Windows {
        volume: u64::MAX,
        file_id: [0xfd; 16],
        creation_ticks: 123,
    };
    let raw = serde_json::to_vec(&epoch).unwrap();
    assert_eq!(
        serde_json::from_slice::<IndexedFileEpoch>(&raw).unwrap(),
        epoch
    );
}
#[test]
fn immutable_process_receipt_checks_digest_and_stable_target() {
    let receipt = ProcessJobPublicationReceipt::new(
        "job".into(),
        input(),
        "snapshot".into(),
        "revision".into(),
        "run".into(),
        (1, 100),
    )
    .unwrap();
    let raw = serde_json::to_vec(&receipt).unwrap();
    assert_eq!(
        serde_json::from_slice::<ProcessJobPublicationReceipt>(&raw).unwrap(),
        receipt
    );
    let mut v = serde_json::to_value(receipt).unwrap();
    v["input_sha256"] = serde_json::json!("0".repeat(64));
    assert!(serde_json::from_value::<ProcessJobPublicationReceipt>(v).is_err());
}
#[test]
fn startup_identity_includes_generation_and_domain() {
    let a = ProcessStartupIdentity::Linux {
        pid: 7,
        start_ticks: 10,
        visibility_domain_sha256: [3; 32],
    };
    let b = ProcessStartupIdentity::Linux {
        pid: 7,
        start_ticks: 11,
        visibility_domain_sha256: [3; 32],
    };
    assert_ne!(
        a.canonical_key(input().server_id()),
        b.canonical_key(input().server_id())
    );
    assert!(
        ProcessStartupIdentity::Linux {
            pid: 7,
            start_ticks: 0,
            visibility_domain_sha256: [3; 32]
        }
        .validate()
        .is_err()
    );
}
#[test]
fn summary_is_per_resource_positive_and_partial_has_fixed_codes() {
    let p = ProcessStartupIdentity::Linux {
        pid: 7,
        start_ticks: 10,
        visibility_domain_sha256: [3; 32],
    };
    let summary = ProcessEvidenceSummary::new(
        ProcessObservationMethod::LinuxProcfsV1,
        (10, 11),
        ProcessObservationCoverage::Partial,
        [3; 32],
        vec![p.clone()],
        vec![ProcessObservationCode::VisibilityRestricted],
    )
    .unwrap();
    assert_eq!(
        serde_json::from_value::<ProcessEvidenceSummary>(serde_json::to_value(&summary).unwrap())
            .unwrap(),
        summary
    );
    assert_ne!(summary.observation_fingerprint(&input()), input().digest());
    assert!(
        ProcessEvidenceSummary::new(
            ProcessObservationMethod::LinuxProcfsV1,
            (10, 11),
            ProcessObservationCoverage::Partial,
            [3; 32],
            vec![],
            vec![]
        )
        .is_err()
    );
    assert!(
        ProcessEvidenceSummary::new(
            ProcessObservationMethod::LinuxProcfsV1,
            (10, 11),
            ProcessObservationCoverage::VisibleMethodDomainComplete,
            [4; 32],
            vec![p],
            vec![]
        )
        .is_err()
    );
}
#[test]
fn unix_observation_decode_rechecks_private_fields() {
    let original =
        UnixFileObservation::new(epoch(), 0o100600, 8, (1, 2), (3, 4), (10, 11)).unwrap();
    assert_eq!(
        UnixFileObservation::decode(&original.encode().unwrap()).unwrap(),
        original
    );
    let mut v = serde_json::to_value(original).unwrap();
    v["file_mode"] = serde_json::json!(0o040700);
    assert!(serde_json::from_value::<UnixFileObservation>(v).is_err());
}
#[test]
fn limits_are_fixed_finite_and_reject_extra_client_fields() {
    assert!(ProcessEvidenceLimits::new(0, 1, 1, 1, 1, 0, 1).is_err());
    assert!(ProcessEvidenceLimits::new(1, 1, 1, 8, 7, 0, 1).is_err());
    let mut v = serde_json::to_value(ProcessEvidenceLimits::default()).unwrap();
    v["argv"] = serde_json::json!(["secret"]);
    assert!(serde_json::from_value::<ProcessEvidenceLimits>(v).is_err());
}
