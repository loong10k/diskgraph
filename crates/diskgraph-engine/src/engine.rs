//! 共享 Engine 的 engine 职责；原调用与持锁顺序保持。

use crate::EngineError;
use diskgraph_core::{ScanBudget, Watermark};
use diskgraph_store::{ControlStore, SqliteSnapshotStore};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

/// 共享引擎唯一持有两库连接、取消与扫描进度状态，职责模块复用这些状态。
/// 来源：原生 Rust diskgraph-engine::Engine。
/// The shared DiskGraph service: one data directory, two databases, durable jobs.
pub struct Engine {
    pub(super) data_dir: PathBuf,
    pub(super) graph_path: PathBuf,
    pub(super) max_nodes_per_scan: u64,
    pub(super) scan_budget: ScanBudget,
    pub(super) capacity_watermark: Watermark,
    pub(super) max_active_jobs_per_principal: u32,
    pub(super) scan_options: diskgraph_disktree_core::scan::ScanOptions,
    pub(super) graph: Mutex<SqliteSnapshotStore>,
    pub(super) control: Mutex<ControlStore>,
    pub(super) cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub(super) scan_progress:
        Mutex<HashMap<(String, u64), diskgraph_disktree_core::scan::ScanSnapshot>>,
}

impl Engine {
    /// 向可信 Ops/宿主提供既有控制库锁。
    /// 参数：无；调用方负责业务授权和锁顺序。
    /// 返回：同一控制库 guard 或锁中毒错误。
    /// Gives the ops layer access to the control database (plans, approvals,
    /// operations, recovery) without duplicating the file layout.
    pub fn control_store(&self) -> Result<std::sync::MutexGuard<'_, ControlStore>, EngineError> {
        self.control()
    }
}

impl Engine {
    /// 获取唯一控制库互斥锁。
    /// 参数：无；不得在已持有同锁时重入。
    /// 返回：控制 guard 或 Poisoned。
    pub(super) fn control(&self) -> Result<std::sync::MutexGuard<'_, ControlStore>, EngineError> {
        self.control
            .lock()
            .map_or_else(|_| Err(EngineError::Poisoned), Ok)
    }
}

impl Engine {
    /// 获取唯一图写连接互斥锁。
    /// 参数：无；与控制锁组合时保留 graph→control 顺序。
    /// 返回：图 guard 或 Poisoned。
    pub(super) fn graph(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, SqliteSnapshotStore>, EngineError> {
        self.graph
            .lock()
            .map_or_else(|_| Err(EngineError::Poisoned), Ok)
    }
}

impl Engine {
    /// 访问每个任务当前代次的取消标志。
    /// 参数：无。
    /// 返回：取消表 guard 或 Poisoned。
    pub(super) fn cancellations(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>, EngineError> {
        self.cancellations
            .lock()
            .map_or_else(|_| Err(EngineError::Poisoned), Ok)
    }
}
