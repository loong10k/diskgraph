//! 累计成功、失败和未核验结果以及所有实际读取成本。

/// 累计成功、失败和未核验结果以及所有实际读取成本。
/// 来源：原生 Rust diskgraph-engine::verify::VerifySummary。
/// What a verification pass did, and what it could not do.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct VerifySummary {
    /// 已尝试核验的文件对，任一侧失败也消耗文件预算。
    pub attempted_files: u64,
    /// Files whose contents were read on both sides and matched.
    pub confirmed_same: u64,
    /// Files whose contents were read and differ.
    pub confirmed_different: u64,
    /// Files left unverified: over the budget, unreadable, or a placeholder.
    pub unverified: u64,
    /// Bytes read across every file, on both sides.
    pub bytes_read: u64,
}

impl VerifySummary {
    /// 检查本轮是否还有未核验文件。
    /// 参数：无。
    /// 返回：unverified 为零时 true；仅描述本轮范围。
    /// Whether anything was left unsaid, which a caller must not mistake for
    /// "everything checked out".
    pub fn is_complete(&self) -> bool {
        self.unverified == 0
    }
}
