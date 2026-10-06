use crate::macos_epoch_floor::MacosEpochFloor;
use crate::macos_host_settings::MacosHostSettings;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
fn settings() -> Value {
    json!({"schema_version":1,"public_key":SigningKey::from_bytes(&[81;32]).verifying_key().to_bytes().to_vec(),
        "active_epoch":9,"epoch_floor":9,"expected_sha256":vec![2_u8;32],"expected_bytes":100,
        "installation_root":b"/Library/DiskGraph/versions".to_vec(),
        "receipt_path":b"/Library/DiskGraph/versions/9/receipt.json".to_vec()})
}
#[test]
fn independent_floor_rejects_rollback_and_every_changed_trust_field() {
    let value = settings();
    let original = MacosHostSettings::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    let floor = MacosEpochFloor::decode(
        &serde_json::to_vec(&json!({"schema_version":1,
        "epoch":9,"active_sha256":original.binding_digest().unwrap()}))
        .unwrap(),
    )
    .unwrap();
    floor.verify(&original).unwrap();
    let newer_floor = MacosEpochFloor::decode(
        &serde_json::to_vec(&json!({"schema_version":1,
        "epoch":10,"active_sha256":original.binding_digest().unwrap()}))
        .unwrap(),
    )
    .unwrap();
    assert!(newer_floor.verify(&original).is_err());
    for (key, replacement) in [
        ("active_epoch", json!(8)),
        ("expected_bytes", json!(101)),
        ("expected_sha256", json!(vec![3_u8; 32])),
        (
            "public_key",
            json!(
                SigningKey::from_bytes(&[82; 32])
                    .verifying_key()
                    .to_bytes()
                    .to_vec()
            ),
        ),
        ("installation_root", json!(b"/Library/DiskGraph".to_vec())),
        (
            "receipt_path",
            json!(b"/Library/DiskGraph/versions/9/other.json".to_vec()),
        ),
    ] {
        let mut changed = value.clone();
        changed[key] = replacement;
        if key == "active_epoch" {
            changed["epoch_floor"] = json!(8);
        }
        let changed = MacosHostSettings::decode(&serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(floor.verify(&changed).is_err(), "{key}");
    }
}
#[test]
fn floor_rejects_duplicate_unknown_zero_and_oversized_records() {
    for raw in [
        r#"{"schema_version":1,"epoch":0,"active_sha256":[]}"#,
        r#"{"schema_version":1,"epoch":9,"epoch":9,"active_sha256":[]}"#,
        r#"{"schema_version":2,"epoch":9,"active_sha256":[]}"#,
    ] {
        assert!(MacosEpochFloor::decode(raw.as_bytes()).is_err());
    }
    let base = json!({"schema_version":1,"epoch":9,"active_sha256":vec![2_u8;32]});
    let mut extra = base.clone();
    extra["unknown"] = json!(1);
    assert!(MacosEpochFloor::decode(&serde_json::to_vec(&extra).unwrap()).is_err());
    assert!(MacosEpochFloor::decode(&vec![b' '; 4097]).is_err());
    let serialized = serde_json::to_string(&base).unwrap();
    let duplicate = serialized.replacen('{', "{\"epoch\":9,", 1);
    assert!(MacosEpochFloor::decode(duplicate.as_bytes()).is_err());
    let mut bad_schema = base.clone();
    bad_schema["schema_version"] = json!(2);
    assert!(MacosEpochFloor::decode(&serde_json::to_vec(&bad_schema).unwrap()).is_err());
    let mut zero = base;
    zero["epoch"] = json!(0);
    assert!(MacosEpochFloor::decode(&serde_json::to_vec(&zero).unwrap()).is_err());
}
#[test]
fn binding_is_independent_of_json_whitespace_and_order() {
    let value = settings();
    let reversed = format!(
        "{{{}}}",
        value
            .as_object()
            .unwrap()
            .iter()
            .rev()
            .map(|(key, value)| format!("{}:{}", serde_json::to_string(key).unwrap(), value))
            .collect::<Vec<_>>()
            .join(",")
    );
    let reordered = MacosHostSettings::decode(reversed.as_bytes()).unwrap();
    let compact = MacosHostSettings::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    let pretty =
        MacosHostSettings::decode(serde_json::to_string_pretty(&value).unwrap().as_bytes())
            .unwrap();
    assert_eq!(
        compact.binding_digest().unwrap(),
        reordered.binding_digest().unwrap()
    );
    assert_eq!(
        compact.binding_digest().unwrap(),
        pretty.binding_digest().unwrap()
    );
}

#[test]
fn canonical_binding_has_versioned_fixed_vector() {
    let mut value = settings();
    value["public_key"] = json!([
        215, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100, 7, 58, 14, 225, 114, 243,
        218, 166, 35, 37, 175, 2, 26, 104, 247, 7, 81, 26
    ]);
    let parsed = MacosHostSettings::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        parsed.binding_digest().unwrap(),
        [
            13, 54, 204, 122, 22, 68, 99, 122, 142, 247, 125, 49, 176, 59, 125, 35, 34, 10, 58,
            252, 203, 56, 251, 250, 51, 139, 231, 2, 39, 181, 28, 171
        ]
    );
}
