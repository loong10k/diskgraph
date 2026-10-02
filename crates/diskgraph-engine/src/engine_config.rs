//! 配置数据目录、扫描/任务/容量预算及上游扫描选项。

use diskgraph_core::{ScanBudget, Watermark};
use std::path::PathBuf;

/// 配置数据目录、扫描/任务/容量预算及上游扫描选项。
/// 来源：原生 Rust diskgraph-engine::EngineConfig。
/// Engine construction options; the data directory holds both databases.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub data_dir: PathBuf,
    /// 可信旧 FFI 数据库文件兼容入口；默认使用 data_dir/diskgraph.sqlite。
    pub graph_database_path: Option<PathBuf>,
    /// Hard node budget per scan (RT-04). Exceeding it fails the job and
    /// never publishes a partial latest revision.
    pub max_nodes_per_scan: u64,
    /// Maximum active (queued or running) jobs one principal may hold
    /// (P4 task 5.5, spec MCP-06). Excess requests are refused with
    /// `resource_exhausted` instead of queueing without bound.
    pub max_active_jobs_per_principal: u32,
    /// The walk budget: nodes, duration, staging bytes, and write batch size
    /// (P1 task 2.9, spec RT-02). Reaching a hard limit stops the walk for a
    /// named reason instead of returning less data without saying so.
    pub scan_budget: ScanBudget,
    /// Where the engine refuses new work once the data directory fills up
    /// (P1 task 2.12, spec RT-04). A refusal never deletes anything.
    pub capacity_watermark: Watermark,
    /// How the walk behaves, using disktree's own option contract: a scope
    /// indexed with one set of options is only ever comparable with a scope
    /// indexed the same way (the snapshot records these verbatim).
    pub scan_options: diskgraph_disktree_core::scan::ScanOptions,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("diskgraph-data"),
            graph_database_path: None,
            max_nodes_per_scan: 2_000_000,
            max_active_jobs_per_principal: 8,
            scan_budget: ScanBudget {
                max_nodes: 2_000_000,
                ..ScanBudget::default()
            },
            capacity_watermark: Watermark {
                warn_above_bytes: 8 << 30,
                refuse_above_bytes: 16 << 30,
            },
            scan_options: diskgraph_disktree_core::scan::ScanOptions::default(),
        }
    }
}
