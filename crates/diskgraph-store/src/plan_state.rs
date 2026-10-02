use serde::{Deserialize, Serialize};

/// 只能失效或消费的计划生命周期。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// Plan lifecycle. A plan is never edited, only invalidated or consumed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
    Validated,
    Expired,
    Revoked,
    Applied,
}

impl PlanState {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    /// 编码或解析既有稳定 wire 标签。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：该生命周期/结果枚举既有 snake_case 标签。
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::Validated => "validated",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Applied => "applied",
        }
    }

    /// 编码或解析既有稳定 wire 标签。
    /// 参数：value：待编码/解析字段。
    /// 返回：已知生命周期/结果标签对应枚举，未知为 None。
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "validated" => Self::Validated,
            "expired" => Self::Expired,
            "revoked" => Self::Revoked,
            "applied" => Self::Applied,
            _ => return None,
        })
    }
}
