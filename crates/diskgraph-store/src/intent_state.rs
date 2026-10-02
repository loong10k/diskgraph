use serde::{Deserialize, Serialize};

/// 文件副作用前持久化的执行意图状态。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// Per-item progress. `intent` is written before the file changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    /// Nothing attempted yet.
    Pending,
    /// Intent recorded; the mutation may or may not have happened.
    IntentRecorded,
    /// The item completed with a recorded result.
    Done,
    /// The item failed; `detail` says why.
    Failed,
    /// The item was cancelled before it ran.
    Cancelled,
}

impl IntentState {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    /// 编码或解析既有稳定 wire 标签。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：该生命周期/结果枚举既有 snake_case 标签。
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::IntentRecorded => "intent_recorded",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// 编码或解析既有稳定 wire 标签。
    /// 参数：value：待编码/解析字段。
    /// 返回：已知生命周期/结果标签对应枚举，未知为 None。
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "intent_recorded" => Self::IntentRecorded,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}
