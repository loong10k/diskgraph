use serde::{Deserialize, Serialize};

/// 明确的恢复方式，不把永久删除当作恢复策略。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// The only acceptable recovery stories; "delete" is deliberately absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryRule {
    /// Moved into a same-volume quarantine with a restore record.
    Quarantine,
    /// The action is reversible by moving back to the original location.
    MoveBack,
    /// Copying: the source is untouched, so nothing needs undoing.
    NoneNeeded,
}
