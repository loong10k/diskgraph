use diskgraph_disktree_core::{scan, tree::Metric};
use serde::{Deserialize, Serialize};
use std::io;

/// 原 pinned ScanOptions 的全部选项；来源：disktree scan.rs 与 tree::Metric。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanOptions {
    pub apparent_size: bool,
    pub follow_links: bool,
    pub include_hidden: bool,
    pub one_filesystem: bool,
    pub max_depth: Option<u64>,
    pub dedup_hardlinks: bool,
    pub metric: u8,
}

impl ScanOptions {
    /// 从真实上游选项投影。参数 options 不变；返回无语义默认值替换的线格式。
    /// 参数：options 为真实 pinned ScanOptions 的借用。
    /// 返回：保留全部七项选项的传输记录。
    pub fn from_native(options: &scan::ScanOptions) -> Self {
        Self {
            apparent_size: options.apparent_size,
            follow_links: options.follow_links,
            include_hidden: options.include_hidden,
            one_filesystem: options.one_filesystem,
            max_depth: options.max_depth.map(|depth| depth as u64),
            dedup_hardlinks: options.dedup_hardlinks,
            metric: match options.metric {
                Metric::Bytes => 0,
                Metric::Files => 1,
            },
        }
    }

    /// 恢复上游选项。返回全部原字段；未知 metric 或无法表示的深度拒绝。
    /// 参数：self 为传输中的扫描选项。
    /// 返回：原生 ScanOptions；未知 metric 或本机无法表示的深度拒绝。
    pub fn to_native(&self) -> io::Result<scan::ScanOptions> {
        Ok(scan::ScanOptions {
            apparent_size: self.apparent_size,
            follow_links: self.follow_links,
            include_hidden: self.include_hidden,
            one_filesystem: self.one_filesystem,
            max_depth: self
                .max_depth
                .map(usize::try_from)
                .transpose()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "depth overflow"))?,
            dedup_hardlinks: self.dedup_hardlinks,
            metric: match self.metric {
                0 => Metric::Bytes,
                1 => Metric::Files,
                _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown metric")),
            },
        })
    }
}
