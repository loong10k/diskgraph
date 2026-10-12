use super::BrokerQuery;
use super::broker_query_request::BrokerQueryRequest;
use std::time::{Duration, Instant};

#[test]
fn fixed_mcp_session_has_no_client_executable_environment_or_identity_options() {
    let query = BrokerQuery::new("mcp", None, vec![], vec![]).unwrap();
    assert!(query.requires_customer_executor());
    for key in [
        "data-dir",
        "profile",
        "transport",
        "principal",
        "executable",
    ] {
        assert!(BrokerQuery::new("mcp", None, vec![(key, "foreign".into())], vec![]).is_err());
    }
}

#[test]
fn scan_budgets_and_flags_are_validated_without_allowing_identity_overrides() {
    for command in ["index", "sync"] {
        let query = BrokerQuery::new(
            command,
            None,
            vec![
                ("scope", "a".into()),
                ("max-nodes-per-scan", "123".into()),
                ("max-staging-bytes", "456789".into()),
                ("depth", "7".into()),
            ],
            vec![
                "wait",
                "apparent-size",
                "no-hidden",
                "one-filesystem",
                "cross-filesystems",
                "no-dedup-hardlinks",
            ],
        );
        assert!(query.is_ok(), "fixed scan settings were rejected");
        for key in ["max-nodes-per-scan", "max-staging-bytes"] {
            for value in ["-1", "NaN", "18446744073709551616", "--principal=admin"] {
                assert!(
                    BrokerQuery::new(
                        command,
                        None,
                        vec![("scope", "a".into()), (key, value.into())],
                        vec![]
                    )
                    .is_err()
                );
            }
        }
    }
}

#[test]
fn query_refuses_global_overrides_unknown_switches_duplicates_and_invalid_numbers() {
    for options in [
        vec![("data-dir", "/foreign".into())],
        vec![("principal", "administrator".into())],
        vec![("scope", "a".into()), ("scope", "b".into())],
        vec![("scope", "a".into()), ("parent-id", "-1".into())],
    ] {
        assert!(BrokerQuery::new("top", None, options, Vec::new()).is_err());
    }
    assert!(BrokerQuery::new("node", None, vec![("scope", "a".into())], vec!["apply"]).is_err());
    assert!(
        BrokerQuery::new(
            "scope",
            Some("remove"),
            vec![("scope", "a".into())],
            Vec::new()
        )
        .is_err()
    );
    assert!(
        BrokerQuery::new("index", None, vec![("scope", "a".into())], Vec::new())
            .unwrap()
            .requires_customer_executor()
    );
    assert!(
        BrokerQuery::new(
            "search",
            None,
            vec![("scope", "a".into()), ("pattern", "x\0y".into())],
            Vec::new()
        )
        .is_err()
    );
}

#[test]
fn query_values_stay_in_one_argument_and_keep_original_unicode() {
    let query = BrokerQuery::new(
        "search",
        None,
        vec![
            ("scope", "--principal=attacker".into()),
            ("pattern", "İSTANBUL\n--data-dir=/foreign".into()),
        ],
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        query.arguments().unwrap(),
        [
            "search",
            "--scope=--principal=attacker",
            "--pattern=İSTANBUL\n--data-dir=/foreign"
        ]
    );
}

#[test]
fn query_deserialization_cannot_bypass_birth_validation() {
    let query: BrokerQuery = serde_json::from_value(serde_json::json!({
        "command":"node", "subcommand":null, "options":[["scope","a"],["principal","admin"]], "switches":[]
    })).unwrap();
    assert!(query.arguments().is_err());
    assert!(
        serde_json::from_value::<BrokerQuery>(serde_json::json!({
            "command":"doctor", "subcommand":null, "options":[], "switches":[], "environment":{}
        }))
        .is_err()
    );
}

#[test]
fn query_wire_keeps_original_deadline_and_rejects_unknown_versions_and_fields() {
    let query = BrokerQuery::new("scope", Some("list"), Vec::new(), Vec::new()).unwrap();
    let original = Instant::now() + Duration::from_secs(5);
    let bytes = BrokerQueryRequest::encode(&query, original).unwrap();
    let acceptance = Instant::now() + Duration::from_secs(1);
    let (decoded, deadline) = BrokerQueryRequest::decode(&bytes, acceptance).unwrap();
    assert_eq!(decoded, query);
    assert!(deadline <= acceptance && deadline <= original);
    assert!(BrokerQueryRequest::decode(b"DG_FRONT_QUERY_V0\n{}", acceptance).is_err());
    let payload = bytes.strip_prefix(b"DG_FRONT_QUERY_V1\n").unwrap();
    let mut object: serde_json::Value = serde_json::from_slice(payload).unwrap();
    object["environment"] = serde_json::json!({"FD":"5"});
    let mut injected = b"DG_FRONT_QUERY_V1\n".to_vec();
    injected.extend(serde_json::to_vec(&object).unwrap());
    assert!(BrokerQueryRequest::decode(&injected, acceptance).is_err());
    assert!(BrokerQueryRequest::decode(&vec![b'x'; 4097], acceptance).is_err());
}

#[test]
fn customer_scan_commands_use_fixed_scope_and_wait_without_collector_or_global_overrides() {
    for command in ["index", "sync"] {
        let query =
            BrokerQuery::new(command, None, vec![("scope", "a".into())], vec!["wait"]).unwrap();
        assert_eq!(query.arguments().unwrap(), [command, "--scope=a", "--wait"]);
        for key in ["collector", "data-dir", "principal"] {
            assert!(
                BrokerQuery::new(
                    command,
                    None,
                    vec![("scope", "a".into()), (key, "foreign".into())],
                    vec![]
                )
                .is_err()
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn native_scope_root_preserves_non_utf8_bytes_and_cannot_be_attached_to_other_commands() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/raw-\xff".to_vec()));
    let query = BrokerQuery::for_native_root(&path).unwrap();
    assert!(query.requires_customer_executor());
    assert!(query.arguments().is_err());
    assert_eq!(
        query.native_arguments().unwrap()[2].as_os_str().as_bytes(),
        b"--root=/tmp/raw-\xff"
    );
    let mut wire = serde_json::to_value(&query).unwrap();
    wire["native_root"]["display"] = "--principal=administrator".into();
    let decoded: BrokerQuery = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        decoded.native_arguments().unwrap(),
        query.native_arguments().unwrap()
    );
    wire["command"] = "doctor".into();
    wire["subcommand"] = serde_json::Value::Null;
    assert!(
        serde_json::from_value::<BrokerQuery>(wire)
            .unwrap()
            .validate()
            .is_err()
    );
    for path in [
        std::path::PathBuf::from("relative"),
        std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/a\0b".to_vec())),
    ] {
        assert!(BrokerQuery::for_native_root(&path).is_err());
    }
}
