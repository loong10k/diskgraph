use super::LinuxExecutorRequest;
use crate::native_deadline::ClockStamp;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn request() -> Value {
    json!({"schema_version":1,"uid":1001,"gid":1002,"group_count":2,
        "groups_sha256":"ab".repeat(32),"worker_fd":7,"control_fd":6,
        "worker_sha256":"cd".repeat(32),"worker_bytes":512,
        "data_dir":"/var/lib/diskgraph/user-data",
        "deadline":ClockStamp::capture(Instant::now()+Duration::from_secs(5)).unwrap()})
}

#[test]
fn executor_request_has_no_capacity_or_namespace_material() {
    let parsed = LinuxExecutorRequest::decode(&serde_json::to_vec(&request()).unwrap()).unwrap();
    assert_eq!(parsed.identity(), (1001, 1002));
    assert_eq!(parsed.group_binding(), (2, [0xab; 32]));
    assert_eq!(
        parsed.data_dir(),
        std::path::Path::new("/var/lib/diskgraph/user-data")
    );
    assert_eq!(parsed.worker_expectation().unwrap().expected_bytes, 512);
    assert!(parsed.deadline().unwrap() > Instant::now());
    for key in [
        "state_root",
        "slot_fd",
        "namespace_fd",
        "service_uid",
        "frontend_uid",
        "worker_path",
        "trusted_local",
        "command",
    ] {
        let mut value = request();
        value[key] = json!("injected");
        assert!(
            LinuxExecutorRequest::decode(&serde_json::to_vec(&value).unwrap()).is_err(),
            "accepted {key}"
        );
    }
}

#[test]
fn executor_identity_and_fixed_descriptors_cannot_be_replaced() {
    for (key, invalid) in [
        ("uid", json!(0)),
        ("uid", json!(u32::MAX)),
        ("gid", json!(0)),
        ("gid", json!(u32::MAX)),
        ("worker_fd", json!(6)),
        ("worker_fd", json!(8)),
        ("control_fd", json!(5)),
        ("control_fd", json!(7)),
        ("schema_version", json!(0)),
        ("schema_version", json!(2)),
        ("group_count", json!(65537)),
        ("groups_sha256", json!("x".repeat(64))),
        ("worker_bytes", json!(0)),
        ("worker_bytes", json!((64_u64 << 20) + 1)),
        ("worker_sha256", json!("CD".repeat(32))),
        ("data_dir", json!("relative")),
        ("data_dir", json!("/var/../data")),
        ("data_dir", json!("/var//data")),
        ("data_dir", json!("/var/data\0tail")),
    ] {
        let mut value = request();
        value[key] = invalid;
        assert!(
            LinuxExecutorRequest::decode(&serde_json::to_vec(&value).unwrap()).is_err(),
            "accepted {key}"
        );
    }
}

#[test]
fn missing_binding_or_oversized_request_is_not_a_legacy_fallback() {
    for key in [
        "uid",
        "gid",
        "group_count",
        "groups_sha256",
        "worker_fd",
        "control_fd",
        "deadline",
    ] {
        let mut value = request();
        value.as_object_mut().unwrap().remove(key);
        assert!(LinuxExecutorRequest::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let mut bytes = serde_json::to_vec(&request()).unwrap();
    bytes.extend(vec![b' '; 4096]);
    assert!(LinuxExecutorRequest::decode(&bytes).is_err());
}

#[test]
fn expired_original_clock_is_rejected_without_refreshing_budget() {
    let mut value = request();
    value["deadline"]["expires_nanos"] = json!(0);
    assert!(LinuxExecutorRequest::decode(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn duplicate_identity_fields_are_rejected_instead_of_last_value_winning() {
    let encoded = serde_json::to_string(&request()).unwrap();
    let duplicate = format!("{{\"uid\":9999,{}", &encoded[1..]);
    assert!(LinuxExecutorRequest::decode(duplicate.as_bytes()).is_err());
}

#[test]
fn trusted_host_encoding_binds_full_groups_image_and_original_deadline() {
    let until = Instant::now() + Duration::from_secs(5);
    let image = crate::ScanWorkerHostConfig::from_expected_image([0xcd; 32], 512).unwrap();
    let bytes = LinuxExecutorRequest::encode_from_host(
        (1001, 1002),
        &[2000, 3000],
        std::path::Path::new("/var/lib/diskgraph/user-data"),
        &image,
        until,
    )
    .unwrap();
    let parsed = LinuxExecutorRequest::decode(&bytes).unwrap();
    assert_eq!(parsed.identity(), (1001, 1002));
    assert_eq!(parsed.group_binding().0, 2);
    assert_eq!(
        parsed.worker_expectation().unwrap().expected_sha256,
        [0xcd; 32]
    );
    assert!(parsed.deadline().unwrap() <= until);
    let changed = LinuxExecutorRequest::encode_from_host(
        (1001, 1002),
        &[3000, 2000],
        parsed.data_dir(),
        &image,
        until,
    )
    .unwrap();
    assert_ne!(
        parsed.group_binding().1,
        LinuxExecutorRequest::decode(&changed)
            .unwrap()
            .group_binding()
            .1
    );
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["worker_fd"], 7);
    assert_eq!(value["control_fd"], 6);
    assert!(value.get("state_root").is_none());
    assert!(value.get("slot_fd").is_none());
}

#[test]
fn trusted_host_encoding_refuses_invalid_material_before_birth() {
    let image = crate::ScanWorkerHostConfig::from_expected_image([0xcd; 32], 512).unwrap();
    let path = std::path::Path::new("/var/lib/diskgraph/user-data");
    let until = Instant::now() + Duration::from_secs(5);
    for identity in [(0, 1002), (1001, 0), (u32::MAX, 1002)] {
        assert!(
            LinuxExecutorRequest::encode_from_host(identity, &[], path, &image, until).is_err()
        );
    }
    assert!(
        LinuxExecutorRequest::encode_from_host((1001, 1002), &[u32::MAX], path, &image, until)
            .is_err()
    );
    assert!(
        LinuxExecutorRequest::encode_from_host(
            (1001, 1002),
            &vec![1002; 65537],
            path,
            &image,
            until
        )
        .is_err()
    );
    assert!(
        LinuxExecutorRequest::encode_from_host(
            (1001, 1002),
            &[],
            std::path::Path::new("/var/../data"),
            &image,
            until
        )
        .is_err()
    );
    assert!(
        LinuxExecutorRequest::encode_from_host(
            (1001, 1002),
            &[],
            path,
            &image,
            Instant::now() - Duration::from_secs(1)
        )
        .is_err()
    );
}
