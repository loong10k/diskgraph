//! 拥有单一路径的比较结果、文件标志与可选双侧摘要，不借用完整树。

/// 拥有单一路径的比较结果、文件标志与可选双侧摘要，不借用完整树。
/// 来源：原生 Rust diskgraph-engine::CompareRow。
/// One path's comparison, owned so a report outlives the graphs it came from.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CompareRow {
    pub path: String,
    pub verdict: diskgraph_core::Verdict,
    pub left_bytes: Option<u64>,
    pub right_bytes: Option<u64>,
    /// Whether the path is a file rather than a directory. Recorded by the
    /// comparison that read the node, because a content pass would otherwise
    /// have to guess from the name, and a directory's verdict is a statement
    /// about what it holds rather than about bytes anyone can hash.
    pub is_file: bool,
    /// The content digest each side hashed to, when the comparison was
    /// verified. Carrying the value rather than only the verdict is what
    /// lets a caller see *why* two files were called identical, and reuse the
    /// hash instead of reading both files again to find that out.
    pub digests: Option<(String, String)>,
}
