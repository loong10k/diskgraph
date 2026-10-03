use super::{DifferentReason, Evidence};

/// 两侧路径的存在性、差异或达到的相同证据；来源：DiskGraph 原生 Rust compare::Verdict。
/// What is true of one path across two trees.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Verdict {
    /// Present on the comparison's left - the `--from` side - and absent
    /// from its right. Named for the side rather than for the parameter,
    /// because the comparison is a fact about two trees and only the caller
    /// knows which one it called "from".
    #[serde(rename = "from-only")]
    LeftOnly,
    /// Present on the comparison's right - the `--to` side - and absent from
    /// its left.
    #[serde(rename = "to-only")]
    RightOnly,
    /// Present on both, and not the same.
    Different {
        /// The first test that separated them, most specific first.
        reason: DifferentReason,
    },
    /// Present on both, and the same to the depth tested.
    Same { evidence: Evidence },
}

impl Verdict {
    /// Whether the two sides disagree, in the sense a caller cares about:
    /// something would have to be copied or removed.
    /// 参数：无；返回：当前判定是否不是 Same，供兼容调用者判断差异。
    pub fn is_difference(&self) -> bool {
        !matches!(self, Verdict::Same { .. })
    }

    /// The deepest test that was actually run for this verdict.
    /// 参数：无；返回：存在性或元数据证据级别，不把字段一致提升为内容验证。
    pub fn evidence(&self) -> Evidence {
        match self {
            Verdict::LeftOnly | Verdict::RightOnly => Evidence::Presence,
            Verdict::Different { .. } | Verdict::Same { .. } => Evidence::Metadata,
        }
    }
}
