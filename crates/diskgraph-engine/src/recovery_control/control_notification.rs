use serde::{Deserialize, Serialize};
/// 监督方向的通知；来源：PF-06 监督设计，无 Java 对等对象。
/// 前端命令不能使用这个方向声明资源回收；消息本身不代替原 owner 的物理证明。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlNotification {
    Ready {},
    ForegroundEnded { exit_code: i32, panicked: bool },
    CleanupPending {},
    CleanupComplete {},
}
