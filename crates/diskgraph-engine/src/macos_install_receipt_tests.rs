use crate::ScanWorkerHostConfig;
use crate::macos_install_receipt::MacosInstallReceipt;
use crate::macos_installation_trust::MacosInstallationTrust;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde_json::{Value, json};
use std::path::Path;

const ROOT: &str = "/private/diskgraph_fixture";
const DOMAIN: &[u8] = b"diskgraph.macos.installation.receipt\0v1\0";

fn signing_key() -> SigningKey {
    // 仅测试签名种子；不得作为产品默认信任或发行密钥。
    SigningKey::from_bytes(&[73; 32])
}

fn expected() -> ScanWorkerHostConfig {
    ScanWorkerHostConfig::from_expected_image([19; 32], 4096).unwrap()
}

fn trust() -> MacosInstallationTrust {
    MacosInstallationTrust::from_host(signing_key().verifying_key().to_bytes(), 7, Path::new(ROOT))
        .unwrap()
}

fn claims() -> Value {
    json!({
        "schema_version": 1,
        "installation_id": vec![3_u8; 16],
        "epoch": 7,
        "policy_version": 1,
        "target": env!("DISKGRAPH_ENGINE_TARGET"),
        "protocol_version": 2,
        "pinned_scanner_revision": "158f9cc2f0b332194a3ffc5acec47760c99146d8",
        "image_sha256": vec![19_u8; 32],
        "image_bytes": 4096,
        "native_path": format!("{ROOT}/worker").into_bytes(),
        "volume_uuid": vec![5_u8; 16],
        "fsid": [-2, 91],
        "device": 12,
        "inode": 345,
        "birth_seconds": -1,
        "birth_nanoseconds": 456,
        "fresh_from_birth": true
    })
}

fn append_bytes(message: &mut Vec<u8>, bytes: &[u8]) {
    message.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    message.extend_from_slice(bytes);
}

fn byte_array(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect()
}

// 独立发行夹具按固定合同编码，不调用待测 verifier 的编码器。
fn payload(claims: &Value) -> Vec<u8> {
    let mut result = DOMAIN.to_vec();
    result.extend_from_slice(&(claims["schema_version"].as_u64().unwrap() as u32).to_le_bytes());
    result.extend_from_slice(&byte_array(&claims["installation_id"]));
    result.extend_from_slice(&claims["epoch"].as_u64().unwrap().to_le_bytes());
    result.extend_from_slice(&(claims["policy_version"].as_u64().unwrap() as u32).to_le_bytes());
    append_bytes(&mut result, claims["target"].as_str().unwrap().as_bytes());
    result.extend_from_slice(&(claims["protocol_version"].as_u64().unwrap() as u32).to_le_bytes());
    append_bytes(
        &mut result,
        claims["pinned_scanner_revision"]
            .as_str()
            .unwrap()
            .as_bytes(),
    );
    result.extend_from_slice(&byte_array(&claims["image_sha256"]));
    result.extend_from_slice(&claims["image_bytes"].as_u64().unwrap().to_le_bytes());
    append_bytes(&mut result, &byte_array(&claims["native_path"]));
    result.extend_from_slice(&byte_array(&claims["volume_uuid"]));
    for item in claims["fsid"].as_array().unwrap() {
        result.extend_from_slice(&(item.as_i64().unwrap() as i32).to_le_bytes());
    }
    result.extend_from_slice(&claims["device"].as_u64().unwrap().to_le_bytes());
    result.extend_from_slice(&claims["inode"].as_u64().unwrap().to_le_bytes());
    result.extend_from_slice(&claims["birth_seconds"].as_i64().unwrap().to_le_bytes());
    result.extend_from_slice(&(claims["birth_nanoseconds"].as_u64().unwrap() as u32).to_le_bytes());
    result.push(u8::from(claims["fresh_from_birth"].as_bool().unwrap()));
    result
}

fn receipt(claims: Value) -> Vec<u8> {
    let signature = signing_key().sign(&payload(&claims)).to_bytes().to_vec();
    serde_json::to_vec(&json!({"claims": claims, "signature": signature})).unwrap()
}

fn verify(bytes: &[u8]) -> bool {
    MacosInstallReceipt::verify(bytes, &trust(), &expected()).is_ok()
}

#[test]
fn authentic_receipt_preserves_native_non_utf8_path_and_identity() {
    let mut value = claims();
    let mut path = format!("{ROOT}/worker_").into_bytes();
    path.push(255);
    value["native_path"] = json!(path);
    let actual = MacosInstallReceipt::verify(&receipt(value), &trust(), &expected()).unwrap();
    assert_eq!(actual.native_path, path);
    assert_eq!(actual.birth_seconds, -1);
    assert_eq!(actual.fsid, [-2, 91]);
}

#[test]
fn every_claim_is_authenticated_and_json_order_is_irrelevant() {
    let bytes = receipt(claims());
    let base: Value = serde_json::from_slice(&bytes).unwrap();
    for (field, value) in base["claims"].as_object().unwrap() {
        let mut altered = base.clone();
        altered["claims"][field] = match value {
            Value::Array(items) => {
                let mut items = items.clone();
                items[0] = json!(items[0].as_i64().unwrap() + 1);
                json!(items)
            }
            Value::String(text) => json!(format!("{text}x")),
            Value::Bool(boolean) => json!(!boolean),
            Value::Number(number) => json!(number.as_i64().unwrap() + 1),
            _ => panic!("fixture contains only contract fields"),
        };
        assert!(
            !verify(&serde_json::to_vec(&altered).unwrap()),
            "unauthenticated {field}"
        );
    }
    assert!(verify(&serde_json::to_vec_pretty(&base).unwrap()));
}

#[test]
fn valid_signature_cannot_bypass_semantic_bindings() {
    let invalid = [
        ("schema_version", json!(2)),
        ("installation_id", json!(vec![0_u8; 16])),
        ("epoch", json!(6)),
        ("policy_version", json!(2)),
        ("target", json!("other-target")),
        ("protocol_version", json!(1)),
        ("pinned_scanner_revision", json!("other-pin")),
        ("image_sha256", json!(vec![20_u8; 32])),
        ("image_bytes", json!(4097)),
        ("volume_uuid", json!(vec![0_u8; 16])),
        ("birth_nanoseconds", json!(1_000_000_000)),
        ("fresh_from_birth", json!(false)),
    ];
    for (field, value) in invalid {
        let mut altered = claims();
        altered[field] = value;
        assert!(!verify(&receipt(altered)), "accepted {field}");
    }
}

#[test]
fn receipt_rejects_untrusted_signature_and_wrong_signature_length() {
    let mut value: Value = serde_json::from_slice(&receipt(claims())).unwrap();
    value["signature"] = json!(
        SigningKey::from_bytes(&[74; 32])
            .sign(&payload(&claims()))
            .to_bytes()
            .to_vec()
    );
    assert!(!verify(&serde_json::to_vec(&value).unwrap()));
    for length in [0, 63, 65] {
        value["signature"] = json!(vec![0; length]);
        assert!(!verify(&serde_json::to_vec(&value).unwrap()));
    }
}

#[test]
fn signed_paths_must_be_strictly_below_trusted_root() {
    for path in [
        ROOT.to_owned(),
        format!("{ROOT}_evil/worker"),
        "/outside/worker".into(),
        format!("{ROOT}/../worker"),
        format!("{ROOT}/./worker"),
        format!("{ROOT}//worker"),
        format!("{ROOT}/worker/"),
        format!("{ROOT}/worker\0"),
    ] {
        let mut value = claims();
        value["native_path"] = json!(path.into_bytes());
        assert!(!verify(&receipt(value)));
    }
}

#[test]
fn malformed_unknown_duplicate_and_unbounded_receipts_are_rejected() {
    let bytes = receipt(claims());
    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
    value["unknown"] = json!(1);
    assert!(!verify(&serde_json::to_vec(&value).unwrap()));
    value.as_object_mut().unwrap().remove("unknown");
    value["claims"]["unknown"] = json!(1);
    assert!(!verify(&serde_json::to_vec(&value).unwrap()));
    let text = String::from_utf8(bytes).unwrap();
    assert!(!verify(
        text.replacen("\"epoch\":7", "\"epoch\":7,\"epoch\":7", 1)
            .as_bytes()
    ));
    assert!(!verify(
        text.replacen("\"signature\":", "\"signature\":[],\"signature\":", 1)
            .as_bytes()
    ));
    assert!(!verify(&vec![b' '; 16 * 1024 + 1]));
    assert!(!verify(b"{} trailing"));
}

#[test]
fn trust_rejects_weak_keys_zero_epoch_and_ambiguous_roots() {
    for key in [[0; 32], {
        let mut identity = [0; 32];
        identity[0] = 1;
        identity
    }] {
        assert!(MacosInstallationTrust::from_host(key, 7, Path::new(ROOT)).is_err());
    }
    // 以 dalek 实际解码失败选取确定性反例，不假定 [255;32] 是非法点编码。
    let malformed = (0_u8..=255)
        .map(|byte| [byte; 32])
        .find(|key| VerifyingKey::from_bytes(key).is_err())
        .expect("uniform-byte corpus contains an undecodable Edwards point");
    assert!(MacosInstallationTrust::from_host(malformed, 7, Path::new(ROOT)).is_err());
    assert!(
        MacosInstallationTrust::from_host(
            signing_key().verifying_key().to_bytes(),
            0,
            Path::new(ROOT)
        )
        .is_err()
    );
    for root in [
        "/",
        "relative",
        "/private/./fixture",
        "/private/../fixture",
        "/private//fixture",
        "/private/fixture/",
        "/private/fixture\0",
    ] {
        assert!(
            MacosInstallationTrust::from_host(
                signing_key().verifying_key().to_bytes(),
                7,
                Path::new(root)
            )
            .is_err()
        );
    }
    let long = format!("/{}", "x".repeat(1024));
    assert!(
        MacosInstallationTrust::from_host(
            signing_key().verifying_key().to_bytes(),
            7,
            Path::new(&long)
        )
        .is_err()
    );
}

#[test]
fn signed_zero_oversized_and_malformed_claim_lengths_are_rejected() {
    for length in [0, 128 * 1024 * 1024 + 1] {
        let mut value = claims();
        value["image_bytes"] = json!(length);
        assert!(!verify(&receipt(value)));
    }
    let mut value: Value = serde_json::from_slice(&receipt(claims())).unwrap();
    value["claims"]["image_sha256"] = json!([1, 2]);
    assert!(!verify(&serde_json::to_vec(&value).unwrap()));
    value["claims"]["birth_nanoseconds"] = json!(-1);
    assert!(!verify(&serde_json::to_vec(&value).unwrap()));
}
