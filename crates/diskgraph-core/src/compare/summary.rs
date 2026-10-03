/// 比较判定汇总，分别记录未知及深度未访问项；来源：DiskGraph 原生 Rust compare::Summary。
/// Counts per verdict, so a caller can report a shape without walking the
/// whole list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct Summary {
    pub left_only: u64,
    pub right_only: u64,
    pub different: u64,
    pub same: u64,
    /// Entries skipped because one side could not report a size.
    pub unknown: u64,
    /// Entries not visited because the caller bounded the depth.
    pub skipped: u64,
}

impl Summary {
    /// Paths that would have to be copied or removed to make the trees match.
    /// 参数：无；返回：单侧存在和差异数量之和，保留旧统计口径，不代表操作授权。
    pub fn actionable(&self) -> u64 {
        self.left_only + self.right_only + self.different
    }
}
