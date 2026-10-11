use super::LinuxSupervisorBirthRequest;
use crate::native_deadline::ClockStamp;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn request() -> Value {
    json!({"schema_version":1,"nonce":"ab".repeat(32),"service_uid":1000,
        "frontend_uid":1001,"state_root":"/opt/diskgraph/state",
        "data_dir":"/opt/diskgraph/data","worker_path":"/opt/diskgraph/images/diskgraph-scan-worker",
        "worker_sha256":"cd".repeat(32),"worker_bytes":512,
        "deadline":ClockStamp::capture(Instant::now()+Duration::from_secs(5)).unwrap()})
}

#[test]
fn valid_birth_material_preserves_original_configuration() {
    let parsed =
        LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&request()).unwrap()).unwrap();
    assert_eq!(parsed.schema_version, 1);
    assert_eq!(parsed.nonce, "ab".repeat(32));
    assert_eq!(parsed.service_uid, 1000);
    assert_eq!(parsed.frontend_uid, 1001);
    assert_eq!(
        parsed.state_root,
        std::path::Path::new("/opt/diskgraph/state")
    );
    assert_eq!(parsed.data_dir, std::path::Path::new("/opt/diskgraph/data"));
    assert_eq!(
        parsed.worker_path,
        std::path::Path::new("/opt/diskgraph/images/diskgraph-scan-worker")
    );
    assert_eq!(parsed.worker_sha256, "cd".repeat(32));
    assert_eq!(parsed.worker_bytes, 512);
    assert!(parsed.deadline.adopt(Duration::from_secs(30)).unwrap() > Instant::now());
}

#[test]
fn malformed_birth_policy_is_refused_before_touching_paths() {
    for (key, value) in [
        ("schema_version", json!(2)),
        ("service_uid", json!(0)),
        ("service_uid", json!(u32::MAX)),
        ("frontend_uid", json!(0)),
        ("frontend_uid", json!(1000)),
        ("nonce", json!("00")),
        ("nonce", json!("zz".repeat(32))),
        ("worker_sha256", json!("00")),
        ("worker_sha256", json!("GG".repeat(32))),
        ("worker_bytes", json!(0)),
        ("worker_bytes", json!(65_u64 << 20)),
        ("state_root", json!("relative")),
        ("data_dir", json!("/opt/../data")),
        ("worker_path", json!("/opt/./worker")),
        ("client_selected_command", json!("delete")),
    ] {
        let mut value_request = request();
        value_request[key] = value;
        assert!(
            LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&value_request).unwrap())
                .is_err(),
            "accepted {key}"
        );
    }
}

#[test]
fn oversized_birth_json_is_refused_even_when_schema_fields_are_valid() {
    let mut bytes = serde_json::to_vec(&request()).unwrap();
    bytes.extend(vec![b' '; 4096]);
    assert!(LinuxSupervisorBirthRequest::parse(&bytes).is_err());
}

#[test]
fn v2_requires_private_control_descriptor_and_rejects_legacy_downgrade() {
    let mut value = request();
    value["schema_version"] = json!(2);
    value["control_fd"] = json!(9);
    value["service_gid"] = json!(1001);
    assert!(
        LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&value).unwrap()).is_ok(),
        "authenticated v2 control birth must be accepted"
    );
    for fd in [json!(null), json!(0), json!(2), json!(u32::MAX), json!("9")] {
        value["control_fd"] = fd;
        assert!(LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    value["schema_version"] = json!(1);
    value["control_fd"] = json!(9);
    assert!(LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn v2_missing_primary_group_is_refused_instead_of_accepting_unbound_identity() {
    let mut value = request();
    value["schema_version"] = json!(2);
    value["control_fd"] = json!(9);
    assert!(
        LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
        "v2 must bind the dedicated primary group before admission"
    );
}

#[test]
fn v2_privileged_or_reserved_primary_group_is_refused() {
    for gid in [0, u32::MAX] {
        let mut value = request();
        value["schema_version"] = json!(2);
        value["control_fd"] = json!(9);
        value["service_gid"] = json!(gid);
        assert!(LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn v3_explicit_lifecycle_birth_binds_fixed_control_and_primary_group() {
    let mut value = request();
    value["schema_version"] = json!(3);
    value["control_fd"] = json!(6);
    value["service_gid"] = json!(1001);
    let parsed = LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(parsed.schema_version, 3);
    assert_eq!(parsed.control_fd, Some(6));
    assert_eq!(parsed.service_gid, Some(1001));
}

#[test]
fn v3_refuses_changed_control_identity_or_silent_version_downgrade() {
    for (key, invalid) in [
        ("control_fd", json!(5)),
        ("control_fd", json!(9)),
        ("control_fd", json!(null)),
        ("service_gid", json!(0)),
        ("service_gid", json!(null)),
        ("service_uid", json!(0)),
        ("schema_version", json!(1)),
        ("schema_version", json!(4)),
        ("client_selected_command", json!("serve")),
    ] {
        let mut value = request();
        value["schema_version"] = json!(3);
        value["control_fd"] = json!(6);
        value["service_gid"] = json!(1001);
        value[key] = invalid;
        assert!(
            LinuxSupervisorBirthRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "accepted {key}"
        );
    }
}
