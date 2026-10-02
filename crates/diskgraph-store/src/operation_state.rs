use serde::{Deserialize, Serialize};

/// 实际操作生命周期，保留部分结果和待核对状态。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// Operation lifecycle. Distinct from the plan: an operation records real
/// side effects and can end partially.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Queued,
    Revalidating,
    Running,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
    NeedsAttention,
}

impl OperationState {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    /// 编码或解析既有稳定 wire 标签。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：该生命周期/结果枚举既有 snake_case 标签。
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Revalidating => "revalidating",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::NeedsAttention => "needs_attention",
        }
    }

    /// 编码或解析既有稳定 wire 标签。
    /// 参数：value：待编码/解析字段。
    /// 返回：已知生命周期/结果标签对应枚举，未知为 None。
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "revalidating" => Self::Revalidating,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "partial" => Self::Partial,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "needs_attention" => Self::NeedsAttention,
            _ => return None,
        })
    }

    /// 判断当前操作状态是否为终态。
    /// 参数：self：待判断的操作状态。
    /// 返回：是否为终态；受保护的 advance_operation_state 使用此判断阻止回退。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Cancelled | Self::NeedsAttention
        )
    }
}
