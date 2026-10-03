/// 快照不可比的明确原因，不猜测缺失事实；来源：DiskGraph 原生 Rust query::Incompatibility。
/// Why two snapshots cannot be compared; always reported, never guessed away.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Incompatibility {
    DifferentRoot,
    DifferentVolume,
    UnknownVolume,
    DifferentSettings,
    OutOfOrder,
    IncompleteCoverage,
}
