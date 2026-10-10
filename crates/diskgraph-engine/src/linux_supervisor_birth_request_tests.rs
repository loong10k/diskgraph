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
