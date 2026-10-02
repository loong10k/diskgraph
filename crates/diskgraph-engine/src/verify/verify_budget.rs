//! 限定尝试内容比较的文件对数与单侧文件字节上限。

/// 限定尝试内容比较的文件对数与单侧文件字节上限。
/// 来源：原生 Rust diskgraph-engine::verify::VerifyBudget。
/// How far a content verification may go.
#[derive(Clone, Copy, Debug)]
pub struct VerifyBudget {
    /// Files whose contents may be read in one comparison.
    pub max_files: u64,
    /// Bytes one file may be read for. A file larger than this is left
    /// unverified: hashing it would cost more than re-copying it.
    pub max_bytes_per_file: u64,
}

impl Default for VerifyBudget {
    fn default() -> Self {
        Self {
            // Small on purpose. A verification is for the handful of files a
            // caller is about to act on, not for re-hashing a tree.
            max_files: 256,
            max_bytes_per_file: 64 << 20,
        }
    }
}
