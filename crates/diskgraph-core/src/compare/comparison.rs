use super::Verdict;
use crate::DiskNode;

/// 一个相对路径的比较判定及两侧节点；来源：DiskGraph 原生 Rust compare::Comparison。
/// One path's worth of comparison.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Comparison<'a> {
    /// The path relative to the compared root, in display form.
    pub path: String,
    pub verdict: Verdict,
    pub left: Option<&'a DiskNode>,
    pub right: Option<&'a DiskNode>,
}
