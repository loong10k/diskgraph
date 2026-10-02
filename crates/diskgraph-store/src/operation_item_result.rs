use serde::{Deserialize, Serialize};

/// 逐项执行结果，区分成功、跳过、失败和待核对。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// What actually happened to one item.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationItemResult {
    Pending,
    Moved,
    Copied,
    Quarantined,
    Restored,
    /// Permanently removed under an independent purge authority (OP-07).
    Purged,
    Skipped,
    Failed,
}

impl OperationItemResult {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    /// 编码或解析既有稳定 wire 标签。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：该生命周期/结果枚举既有 snake_case 标签。
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Moved => "moved",
            Self::Copied => "copied",
            Self::Quarantined => "quarantined",
            Self::Restored => "restored",
            Self::Purged => "purged",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    /// 编码或解析既有稳定 wire 标签。
    /// 参数：value：待编码/解析字段。
    /// 返回：已知生命周期/结果标签对应枚举，未知为 None。
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "moved" => Self::Moved,
            "copied" => Self::Copied,
            "quarantined" => Self::Quarantined,
            "restored" => Self::Restored,
            "purged" => Self::Purged,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}
