/// 两个已有节点的首要差异或未知原因；来源：DiskGraph 原生 Rust compare::DifferentReason。
/// Why two entries that both exist still differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DifferentReason {
    /// Different bytes, or different size with contents not read.
    Content,
    /// Same content length, different bytes.
    Size,
    /// The same bytes, but a different modification time.
    Timestamp,
    /// The same file, reached by two different paths.
    Path,
    /// One side reported a size the other did not.
    UnknownSize,
    /// A directory whose children disagree; the directory itself may match.
    Contents,
}
