/// 控制帧的固定失败分类；来源：PF-06 监督设计，无 Java 对等对象。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    #[error("control protocol rejected")]
    Protocol,
    #[error("control transmission cancelled")]
    Cancelled,
    #[error("control budget exhausted")]
    Budget,
    #[error("control deadline expired")]
    Deadline,
    #[error("control stream ended without confirmed cleanup")]
    Unconfirmed,
}
