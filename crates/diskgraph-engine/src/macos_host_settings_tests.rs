use crate::macos_host_settings::MacosHostSettings;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
fn valid() -> Value {
    json!({"schema_version":1,"public_key":SigningKey::from_bytes(&[81;32]).verifying_key().to_bytes().to_vec(),
        "active_epoch":9,"epoch_floor":9,"expected_sha256":vec![2_u8;32],"expected_bytes":100,
        "installation_root":b"/Library/Application Support/DiskGraph/scan-worker/versions".to_vec(),
        "receipt_path":b"/Library/Application Support/DiskGraph/scan-worker/versions/9/receipt.json".to_vec()})
}
#[test]
fn protected_settings_preserve_lossless_paths_and_independent_expectations() {
    let original = valid();
    MacosHostSettings::decode(&serde_json::to_vec(&original).unwrap()).unwrap();
    let mut bytes = b"/Library/DiskGraph/".to_vec();
    bytes.push(0xff);
    bytes.extend(b"/receipt.json");
    let mut input = original;
    input["installation_root"] = json!(b"/Library/DiskGraph".to_vec());
    input["receipt_path"] = json!(bytes);
    MacosHostSettings::decode(&serde_json::to_vec(&input).unwrap()).unwrap();
}
#[test]
fn settings_reject_rollback_unknown_fields_and_unsafe_material() {
    for (key, value) in [
        ("active_epoch", json!(8)),
        ("epoch_floor", json!(0)),
        ("schema_version", json!(2)),
        ("expected_bytes", json!(0)),
        ("unknown", json!(1)),
        ("public_key", json!(vec![0_u8; 32])),
        ("receipt_path", json!(b"/tmp/receipt.json".to_vec())),
    ] {
        let mut input = valid();
        input[key] = value;
        assert!(
            MacosHostSettings::decode(&serde_json::to_vec(&input).unwrap()).is_err(),
            "{key}"
        );
    }
}
#[test]
fn settings_reject_duplicate_and_oversized_configuration() {
    let original = serde_json::to_string(&valid()).unwrap();
    let duplicate = original.replacen('{', "{\"active_epoch\":9,", 1);
    assert!(MacosHostSettings::decode(duplicate.as_bytes()).is_err());
    assert!(MacosHostSettings::decode(&vec![b' '; 65537]).is_err());
}
