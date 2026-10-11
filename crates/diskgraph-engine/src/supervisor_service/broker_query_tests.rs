use super::BrokerQuery;
use super::broker_query_request::BrokerQueryRequest;
use std::time::{Duration, Instant};

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
    assert!(BrokerQuery::new("index", None, vec![("scope", "a".into())], Vec::new()).is_err());
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
