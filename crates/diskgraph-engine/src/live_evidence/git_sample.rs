//! 保存本地 Git 状态、stash 与 upstream 差异；无 upstream 不表示已推送。

/// 保存本地 Git 状态、stash 与 upstream 差异；无 upstream 不表示已推送。
/// 来源：原生 Rust diskgraph-engine::live_evidence::GitSample。
/// What a local Git repository says about itself (EC-02, EV-02):
/// dirty state, stashes, and the ahead/behind
/// counts against the configured upstream — which stay `None` (unknown)
/// when there is no upstream, because "no upstream" is not "pushed".
/// The compatibility sampler's offline/configuration isolation is not verified.
/// Existing stash history requires the verifiable files reference backend;
/// unsupported or incomplete observations return an error rather than a count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitSample {
    pub head: Option<String>,
    pub dirty_count: u64,
    pub stash_count: u64,
    pub ahead_of_upstream: Option<u64>,
    pub behind_upstream: Option<u64>,
    pub notes: Vec<String>,
    pub sampled_at_unix_ms: u64,
}
