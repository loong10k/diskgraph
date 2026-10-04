use crate::{BusinessError, JobAuthorityOrigin, JobRequestAuthority, Permission, PrincipalId};

fn remote(expiry: u64, capabilities: Vec<Permission>) -> JobRequestAuthority {
    JobRequestAuthority::authenticated_remote(
        PrincipalId::new("alice").unwrap(),
        "issuer",
        "http",
        capabilities,
        expiry,
    )
    .unwrap()
}

#[test]
fn remote_uses_original_absolute_expiry_without_admission_skew() {
    let authority = remote(100, vec![Permission::IndexWrite]);
    assert_eq!(authority.expires_at_unix_seconds(), Some(100));
    assert!(authority.validate_at(99).is_ok());
    assert_eq!(
        authority.validate_at(100),
        Err(BusinessError::PermissionDenied)
    );
    assert!(!authority.allows(&Permission::IndexWrite, 101));
    assert!(!authority.allows(&Permission::ContentRead, 99));
}

#[test]
fn normalized_capabilities_are_stable_and_empty_remote_is_never_local() {
    let left = remote(
        100,
        vec![
            Permission::IndexWrite,
            Permission::MetadataRead,
            Permission::IndexWrite,
        ],
    );
    let right = remote(100, vec![Permission::MetadataRead, Permission::IndexWrite]);
    assert_eq!(left, right);
    assert_eq!(
        serde_json::to_string(&left).unwrap(),
        serde_json::to_string(&right).unwrap()
    );
    let empty = remote(100, vec![]);
    assert_eq!(empty.origin(), JobAuthorityOrigin::AuthenticatedRemote);
    assert_eq!(empty.capabilities(), Some([].as_slice()));
    assert!(!empty.allows(&Permission::MetadataRead, 99));
}

#[test]
fn trusted_local_is_explicit_and_has_no_secret_or_expiry() {
    let local =
        JobRequestAuthority::trusted_local(PrincipalId::new("local").unwrap(), "stdio").unwrap();
    assert_eq!(local.origin(), JobAuthorityOrigin::TrustedLocal);
    assert_eq!(local.issuer(), None);
    assert_eq!(local.capabilities(), None);
    assert!(local.allows(&Permission::IndexWrite, u64::MAX));
    let encoded = serde_json::to_string(&local).unwrap();
    assert!(!encoded.contains("bearer"));
    assert_eq!(
        serde_json::from_str::<JobRequestAuthority>(&encoded).unwrap(),
        local
    );
}

#[test]
fn durable_remote_roundtrip_keeps_unix_value_and_provenance() {
    let original = remote(
        u64::MAX,
        vec![Permission::ContentRead, Permission::IndexWrite],
    );
    let encoded = serde_json::to_string(&original).unwrap();
    let loaded: JobRequestAuthority = serde_json::from_str(&encoded).unwrap();
    assert_eq!(loaded, original);
    assert_eq!(loaded.principal().as_str(), "alice");
    assert_eq!(loaded.issuer(), Some("issuer"));
    assert_eq!(loaded.transport(), "http");
    assert!(loaded.validate_at(u64::MAX - 1).is_ok());
    assert!(loaded.validate_at(u64::MAX).is_err());
}

#[test]
fn persisted_remote_missing_or_invalid_fields_fail_closed() {
    let original = serde_json::to_value(remote(100, vec![Permission::IndexWrite])).unwrap();
    for (field, replacement) in [
        ("origin", serde_json::json!("legacy_unknown")),
        ("principal", serde_json::json!("bad/principal")),
        ("issuer", serde_json::Value::Null),
        ("transport", serde_json::json!("")),
        ("capability_ceiling", serde_json::Value::Null),
        ("expires_at_unix_seconds", serde_json::Value::Null),
        ("expires_at_unix_seconds", serde_json::json!(-1)),
    ] {
        let mut malformed = original.clone();
        malformed[field] = replacement;
        assert!(
            serde_json::from_value::<JobRequestAuthority>(malformed).is_err(),
            "{field}"
        );
    }
    let mut missing = original.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("expires_at_unix_seconds");
    assert!(serde_json::from_value::<JobRequestAuthority>(missing).is_err());
    let mut secret = original;
    secret["bearer"] = serde_json::json!("must-not-persist");
    assert!(serde_json::from_value::<JobRequestAuthority>(secret).is_err());
}

#[test]
fn local_marker_cannot_carry_remote_fields_or_invalid_principal() {
    let mut local = serde_json::to_value(
        JobRequestAuthority::trusted_local(PrincipalId::new("local").unwrap(), "cli").unwrap(),
    )
    .unwrap();
    for (field, value) in [
        ("issuer", serde_json::json!("issuer")),
        ("capability_ceiling", serde_json::json!([])),
        ("expires_at_unix_seconds", serde_json::json!(100)),
        ("principal", serde_json::json!("")),
    ] {
        let mut invalid = local.clone();
        invalid[field] = value;
        assert!(serde_json::from_value::<JobRequestAuthority>(invalid).is_err());
    }
    local["origin"] = serde_json::json!("authenticated_remote");
    assert!(serde_json::from_value::<JobRequestAuthority>(local).is_err());
}

#[test]
fn capability_input_is_bounded_before_normalization() {
    assert!(
        JobRequestAuthority::authenticated_remote(
            PrincipalId::new("alice").unwrap(),
            "issuer",
            "http",
            vec![Permission::IndexWrite; 65],
            100,
        )
        .is_err()
    );
}
