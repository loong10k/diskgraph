use crate::{QueryBudget, QueryReadBudget, TruncationReason, measure_json_bounded, query_deadline};

#[test]
fn actual_json_encoding_has_exact_limits_for_escape_unicode_and_keys() {
    let value = serde_json::json!({"键":"\0\n\\\"中文🙂".repeat(500)});
    let bytes = serde_json::to_vec(&value).unwrap().len();
    assert_eq!(measure_json_bounded(&value, bytes).unwrap(), Some(bytes));
    assert_eq!(measure_json_bounded(&value, bytes - 1).unwrap(), None);
    assert_eq!(measure_json_bounded(&value, 0).unwrap(), None);
    assert_eq!(
        measure_json_bounded(&value, usize::MAX).unwrap(),
        Some(bytes)
    );
}

#[test]
fn actual_serialize_errors_are_not_a_byte_limit() {
    struct Failure;
    impl serde::Serialize for Failure {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("fixture serialize failure"))
        }
    }
    assert!(
        measure_json_bounded(&Failure, 100)
            .unwrap_err()
            .to_string()
            .contains("fixture serialize failure")
    );
}

#[test]
fn raw_admission_is_checked_atomic_and_cumulative() {
    let budget = QueryBudget {
        max_nodes: 1,
        max_edges: 2,
        max_response_bytes: 10,
        ..QueryBudget::default()
    };
    let mut reads = QueryReadBudget::new(budget, query_deadline(budget).unwrap()).unwrap();
    assert!(reads.admit(1, 1, 5));
    assert!(!reads.admit(0, 1, usize::MAX));
    assert_eq!(reads.remaining_edges(), 1);
    assert_eq!(reads.remaining_raw_bytes(), 5);
    assert_eq!(reads.stopped(), Some(TruncationReason::ByteLimit));
    assert!(!reads.admit(0, 0, 0));
}

#[test]
fn expired_absolute_deadline_is_observed_even_without_rows() {
    let budget = QueryBudget::default();
    let mut reads = QueryReadBudget::new(budget, std::time::Instant::now()).unwrap();
    assert!(!reads.check());
    assert_eq!(reads.stopped(), Some(TruncationReason::Deadline));
    assert!(
        query_deadline(QueryBudget {
            deadline_ms: 0,
            ..budget
        })
        .is_err()
    );
    assert!(
        query_deadline(QueryBudget {
            deadline_ms: 60_000,
            ..budget
        })
        .is_ok()
    );
}
