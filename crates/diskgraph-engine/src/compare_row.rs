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

impl CompareRow {
    /// 按报告现有 wire 字段编码一个路径条目。
    /// 参数：无；使用本条路径及可选双侧摘要。
    /// 返回：兼容 JSON，计量与完整报告共用该表示。
    pub(super) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path":self.path,"verdict":self.verdict,"left_bytes":self.left_bytes,
            "right_bytes":self.right_bytes,"is_file":self.is_file,
            "digests":self.digests.as_ref().map(|(left,right)|serde_json::json!({"left":left,"right":right})),
        })
    }
}
