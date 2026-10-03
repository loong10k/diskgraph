/// 尺寸过滤条件，未知尺寸仍单独可见；来源：DiskGraph 原生 Rust query::SizeFilter。
/// A size filter for listing queries; `Unknown` keeps unscanned or
/// unreported sizes visible instead of silently dropping them (Q-06).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeFilter {
    /// Only nodes with a reported subtree size above the threshold.
    AtLeast(u64),
    /// Only nodes with no reported size (unscanned, denied, or provider-limited).
    UnknownOnly,
}
