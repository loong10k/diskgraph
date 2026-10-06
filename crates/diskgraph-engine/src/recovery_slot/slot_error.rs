/// 监督槽认领的固定失败；来源：PF-06 监督设计，无 Java 对等对象。
#[derive(Debug, thiserror::Error)]
pub enum SlotError {
    #[error("supervisor slot occupied")]
    Busy,
    #[error("supervisor slot cleanup unconfirmed")]
    Unconfirmed,
    #[error("invalid supervisor slot record")]
    InvalidRecord,
    #[error("unsupported supervisor slot object")]
    Unsupported,
    #[error("supervisor slot deadline expired")]
    Deadline,
    #[error("supervisor slot I/O failed: {0}")]
    Io(#[from] std::io::Error),
}
