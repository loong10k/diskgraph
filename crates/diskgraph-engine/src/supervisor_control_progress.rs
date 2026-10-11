use crate::recovery_control::ControlError;

/// 原资源退休与私有确认的独立结果；来源：PF-06，无 Java 对等对象。
/// 只有 Complete 同时表示原 owner 已退休且完成帧完整写入；仍不代替 broker 的实际接收及原 child wait。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupervisorControlProgress {
    /// 原资源仍 Pending，控制错误不能释放原责任。
    Pending {
        control_failure: Option<ControlError>,
    },
    /// 原资源已退休，唯一完成帧已完整写入。
    Complete,
    /// 原资源已退休，但私有通道未确认；broker 不得据此清除全局 bootstrap。
    RetiredUnconfirmed { control_failure: ControlError },
}
