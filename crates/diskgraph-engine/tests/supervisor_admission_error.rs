//! 原监督槽拒绝的稳定外部错误；来源：PF-06，无 Java 对等实现。
use diskgraph_core::{BusinessError, Envelope};
use diskgraph_engine::{EngineError, recovery_slot::SlotError};

#[test]
fn slot_failures_keep_distinct_admission_codes() {
    for (slot, expected, code, exit) in [
        (
            SlotError::Busy,
            BusinessError::ResourceExhausted,
            "resource_exhausted",
            7,
        ),
        (
            SlotError::Unconfirmed,
            BusinessError::RecoveryUnconfirmed,
            "recovery_unconfirmed",
            8,
        ),
        (
            SlotError::InvalidRecord,
            BusinessError::NeedsAttention,
            "needs_attention",
            8,
        ),
        (
            SlotError::Unsupported,
            BusinessError::Unsupported,
            "unsupported",
            6,
        ),
        (
            SlotError::Deadline,
            BusinessError::BudgetExceeded,
            "budget_exceeded",
            7,
        ),
    ] {
        let error = EngineError::from(slot);
        assert!(matches!(error, EngineError::Business(actual) if actual == expected));
        let frame = Envelope::failure(expected, error.to_string()).into_json();
        assert_eq!(frame["ok"], false);
        assert_eq!(frame["error"]["code"], code);
        assert_eq!(frame["error"]["exit_code"], exit);
        assert!(frame.get("data").is_none());
    }
}

#[test]
fn slot_io_retains_original_error_instead_of_parsing_its_message() {
    let error = EngineError::from(SlotError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "recovery_unconfirmed",
    )));
    let EngineError::Io(error) = error else {
        panic!("original I/O was reclassified")
    };
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "recovery_unconfirmed");
}
