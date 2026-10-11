use crate::EngineError;
use crate::recovery_slot::SlotError;
use diskgraph_core::BusinessError;
use std::io;

/// 参数：error为实际root准入错误；返回：保留槽和原期限的稳定业务分类，不解析错误文本。
pub(super) fn classify(error: &io::Error) -> BusinessError {
    if let Some(error) = error
        .get_ref()
        .and_then(|error| error.downcast_ref::<SlotError>())
    {
        return match error {
            SlotError::Busy => BusinessError::ResourceExhausted,
            SlotError::Unconfirmed => BusinessError::RecoveryUnconfirmed,
            SlotError::InvalidRecord => BusinessError::NeedsAttention,
            SlotError::Unsupported => BusinessError::Unsupported,
            SlotError::Deadline => BusinessError::BudgetExceeded,
            SlotError::Io(_) => BusinessError::InternalError,
        };
    }
    if let Some(EngineError::Business(business)) = error
        .get_ref()
        .and_then(|error| error.downcast_ref::<EngineError>())
        .map(EngineError::primary)
    {
        return *business;
    }
    if error.kind() == io::ErrorKind::TimedOut {
        return BusinessError::BudgetExceeded;
    }
    BusinessError::InternalError
}

#[cfg(test)]
mod tests {
    use super::classify;
    use crate::recovery_slot::SlotError;
    use diskgraph_core::BusinessError;
    use std::io;
    #[test]
    fn public_broker_keeps_original_slot_errors_and_rejects_text_impersonation() {
        for error in [
            SlotError::Busy,
            SlotError::Unconfirmed,
            SlotError::InvalidRecord,
            SlotError::Unsupported,
            SlotError::Deadline,
        ] {
            let expected = match crate::EngineError::from(match &error {
                SlotError::Busy => SlotError::Busy,
                SlotError::Unconfirmed => SlotError::Unconfirmed,
                SlotError::InvalidRecord => SlotError::InvalidRecord,
                SlotError::Unsupported => SlotError::Unsupported,
                SlotError::Deadline => SlotError::Deadline,
                _ => unreachable!(),
            }) {
                crate::EngineError::Business(business) => business,
                _ => unreachable!(),
            };
            assert_eq!(classify(&io::Error::other(error)), expected);
        }
        assert_eq!(
            classify(&io::Error::other("supervisor slot cleanup unconfirmed")),
            BusinessError::InternalError
        );
    }
}
