use super::*;
use sha2::Digest;
use std::path::PathBuf;

#[test]
fn sha256_matches_the_known_vectors() {
    fn digest(input: &[u8]) -> String {
        let mut hasher = sha2::Sha256::new();
        hasher.update(input);
        hex::encode(hasher.finalize())
    }
    assert_eq!(
        digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        digest(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    // A multi-block input exercises the streaming path.
    assert_eq!(
        digest(&vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[cfg(unix)]
#[test]
fn stability_is_void_when_the_object_vanishes_mid_read() {
    let workspace = tempfile::TempDir::with_prefix("dg-content-stable-").unwrap();
    let path = workspace.path().join("f");
    std::fs::write(&path, b"v1").unwrap();
    let before = std::fs::symlink_metadata(&path).ok();
    let identity = before.as_ref().and_then(file_identity);
    assert!(identity_stable(&identity, &path));
    // Replaced: a new inode at the same path is a different object.
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"v2").unwrap();
    assert!(!identity_stable(&identity, &path));
    // Gone: the strongest form of unstable.
    std::fs::remove_file(&path).unwrap();
    assert!(!identity_stable(&identity, &path));
}

#[test]
fn the_redacted_log_never_carries_content() {
    let outcome = ReadOutcome {
        requested_path: PathBuf::from("/tmp/secret.txt"),
        offset: 0,
        bytes: b"THE ENTIRE SECRET BODY".to_vec(),
        file_len: 22,
        truncated: false,
        stopped: None,
        observed_at_unix_ms: 1,
    };
    let log = outcome.redacted_log();
    assert!(!log.contains("SECRET BODY"), "{log}");
    assert!(log.contains("secret.txt"));
    assert!(log.contains("len=22"));
}

#[test]
fn metadata_only_export_drops_bytes_and_names_their_count() {
    let outcome = ReadOutcome {
        requested_path: PathBuf::from("/tmp/secret.txt"),
        offset: 0,
        bytes: b"confidential".to_vec(),
        file_len: 12,
        truncated: true,
        stopped: Some(InspectionStop::Unstable),
        observed_at_unix_ms: 1,
    };
    let export = ExportPolicy::MetadataOnly.export_read(&outcome);
    let text = export.to_string();
    assert!(!text.contains("confidential"), "{text}");
    assert_eq!(export["read_len"], 12);
    assert_eq!(export["truncated"], true);
    assert_eq!(export["stopped"], "unstable");
    assert!(export.get("bytes_hex").is_none());
}
