//! 保存两次采样的变化与事件预算溢出，溢出提示受控重扫。

use super::FsEvent;

/// 保存两次采样的变化与事件预算溢出，溢出提示受控重扫。
/// 来源：原生 Rust diskgraph-engine::live_evidence::WatchReport。
/// The diff between two samples of one tree. Polling never drops events
/// (each poll diffs whole-tree snapshots), but a change burst larger than
/// the event budget is an overflow: the report says `rescan_needed` so the
/// caller schedules a controlled rescan instead of trusting a partial view
/// (FS-06).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchReport {
    pub events: Vec<FsEvent>,
    pub overflow: bool,
    pub rescan_needed: bool,
}
