use crate::{IntentState, OperationItemResult};
use serde::{Deserialize, Serialize};

/// 单条资源的执行意图、结果与诊断记录。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// One item's recorded progress.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperationItem {
    pub operation_id: String,
    pub item_index: u32,
    pub intent: IntentState,
    pub result: OperationItemResult,
    pub detail: String,
    pub recovery_ref: Option<String>,
}
