use super::guardian_frontend_binding::GuardianFrontendBinding;

#[test]
fn installed_binding_rejects_wrong_scope_unknown_duplicate_and_alias_identities() {
    let good = r#"{"schema_version":1,"socket_path":"/var/lib/dg/front.sock","frontend_uid":23112,"guardian_uid":23111,"guardian_gid":23113}"#;
    let binding =
        GuardianFrontendBinding::decode(good.as_bytes(), "/var/lib/dg/front.sock").unwrap();
    assert_eq!(binding.frontend_uid(), 23112);
    assert_eq!(binding.guardian_identity(), (23111, 23113));
    assert!(GuardianFrontendBinding::decode(good.as_bytes(), "/foreign").is_err());
    for body in [
        good.replace("23111", "23112"),
        good.replace("23113", "0"),
        good.replace("23111", "4294967295"),
        good.replace("schema_version\":1", "schema_version\":2"),
        good.replace("{", "{\"capacity\":1,"),
        good.replace("{", "{\"frontend_uid\":42,"),
    ] {
        assert!(
            GuardianFrontendBinding::decode(body.as_bytes(), "/var/lib/dg/front.sock").is_err()
        );
    }
    assert!(GuardianFrontendBinding::decode(&vec![b' '; 4097], "/var/lib/dg/front.sock").is_err());
}
