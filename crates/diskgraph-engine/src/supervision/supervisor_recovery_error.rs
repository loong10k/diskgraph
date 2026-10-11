use crate::EngineError;
use crate::recovery_slot::SlotError;
/// 原监督退休诊断；来源：PF-06，不取代原业务错误，无 Java 对等对象。
#[derive(Debug, thiserror::Error)]
pub enum SupervisorRecoveryError {
    #[error("original supervisor resource recovery failed: {0}")]
    Engine(#[from] EngineError),
    #[error("original supervisor slot confirmation failed: {0}")]
    Slot(#[from] SlotError),
}
