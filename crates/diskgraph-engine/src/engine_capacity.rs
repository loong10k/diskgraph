//! 共享 Engine 的 engine_capacity 职责；原调用与持锁顺序保持。

use crate::Engine;
use diskgraph_core::{CapacityReading, StorageArea};
use std::path::Path;
use std::path::PathBuf;

impl Engine {
    /// 计量既有存储区占用及阈值判断。
    /// 参数：无。
    /// 返回：逐区占用/水位；无法遍历时按超限处理。
    /// Per-area capacity readings with each area's verdict.
    pub fn capacity_report(&self) -> Vec<CapacityReading> {
        let readings = vec![
            (StorageArea::GraphDatabase, self.graph_path.clone()),
            (
                StorageArea::ControlDatabase,
                self.data_dir.join("diskgraph-control.sqlite"),
            ),
            (StorageArea::WriteAheadLog, {
                let mut wal = self.graph_path.as_os_str().to_os_string();
                wal.push("-wal");
                PathBuf::from(wal)
            }),
            (StorageArea::Staging, self.data_dir.join("quarantine")),
            (
                StorageArea::Backups,
                self.data_dir.join("migration_backups"),
            ),
            (StorageArea::Logs, self.data_dir.join("logs")),
            (StorageArea::Quarantine, self.data_dir.join("quarantine")),
        ];
        readings
            .into_iter()
            .map(|(area, path)| {
                let used_bytes = directory_bytes(&path);
                let verdict = self.capacity_watermark.verdict(used_bytes);
                CapacityReading {
                    area,
                    used_bytes,
                    watermark: self.capacity_watermark,
                    verdict,
                }
            })
            .collect()
    }
}

impl Engine {
    /// 检查目录/逐区水位和卷可用空间。
    /// 参数：无。
    /// 返回：能否接受新工作；拒绝不删除历史。
    /// Whether the data directory can take new work. A refusal never deletes
    /// anything: existing indexes, control records, and quarantined objects
    /// are left exactly as they are.
    pub fn accepts_new_work(&self) -> bool {
        self.capacity_watermark
            .verdict(directory_bytes(&self.data_dir))
            .accepts_new_work()
            && self
                .capacity_report()
                .iter()
                .all(|reading| reading.verdict.accepts_new_work())
            && volume_headroom(&self.data_dir).is_some_and(|free| free >= 64 * 1024 * 1024)
    }
}

/// 读取卷的可用字节；容量门禁无法测量时拒绝新工作。
fn volume_headroom(path: &Path) -> Option<u64> {
    diskgraph_disktree_core::space::space_info(path)
        .ok()
        .map(|space| space.available)
}
/// Total bytes under a path, or zero when it does not exist.
fn directory_bytes(path: &std::path::Path) -> u64 {
    let mut pending = vec![path.to_path_buf()];
    let mut total = 0u64;
    while let Some(path) = pending.pop() {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return u64::MAX,
        };
        // 容量统计不能沿 quarantine 中的链接扫描 scope 外的数据或循环链接。
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            total = total.saturating_add(metadata.len());
            continue;
        }
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(_) => return u64::MAX,
        };
        for entry in entries {
            match entry {
                Ok(entry) => pending.push(entry.path()),
                Err(_) => return u64::MAX,
            }
        }
    }
    total
}
