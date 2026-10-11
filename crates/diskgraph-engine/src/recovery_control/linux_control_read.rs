use super::ControlFrame;

/// Linux 私有控制单次轮询结果；来源：PF-06 原 child broker 读取，无 Java 对等对象。
/// 协议确认 EOF 不证明原 child wait 或资源退休，不授予 bootstrap 释放资格。
#[derive(Debug, PartialEq, Eq)]
pub enum LinuxControlRead {
    Pending,
    Frame(ControlFrame),
    ConfirmedEof,
}
