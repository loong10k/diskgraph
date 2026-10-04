use crate::ProcessEvidenceLimits;

#[test]
fn process_limits_stream_preserves_canonical_wire_and_numbers() {
    let limits = ProcessEvidenceLimits::new(1, 2, 3, 4, 5, 6, 7).unwrap();
    let raw = serde_json::to_string(&limits).unwrap();
    assert_eq!(
        raw,
        "{\"max_duration_ms\":1,\"max_metadata_bytes\":2,\"max_entries\":3,\"max_result_bytes\":4,\"max_allocation_bytes\":5,\"max_retries\":6,\"max_handles\":7}"
    );
    assert_eq!(
        serde_json::from_str::<ProcessEvidenceLimits>(&raw).unwrap(),
        limits
    );
}

#[test]
fn process_limits_stream_rejects_duplicate_unknown_and_container_fields() {
    let raw = serde_json::to_string(&ProcessEvidenceLimits::default()).unwrap();
    let duplicate = raw.replacen("{", "{\"max_duration_ms\":1,", 1);
    let unknown = raw.replacen("{", "{\"unexpected\":[],", 1);
    let container = raw.replace("\"max_duration_ms\":15000", "\"max_duration_ms\":[15000]");
    for malformed in [duplicate, unknown, container] {
        assert!(serde_json::from_str::<ProcessEvidenceLimits>(&malformed).is_err());
    }
    let invalid = raw.replace("\"max_duration_ms\":15000", "\"max_duration_ms\":0");
    assert!(serde_json::from_str::<ProcessEvidenceLimits>(&invalid).is_err());
}
