//! 有界扫描暂存与失效 fencing 代次清理。

use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::{DiskNode, ResourceLocator};
use rusqlite::params;
use serde_json::to_string;

impl SqliteSnapshotStore {
    /// Appends scanned nodes to invisible staging for a running job; ordinary
    /// queries never read staging, so a crash before publish exposes nothing.
    /// 写入、计数或清理指定代次的扫描暂存数据。
    /// 参数：job_id：本 fencing 代次的暂存命名空间；nodes：有序观测节点集合。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn append_staging_nodes(&mut self, job_id: &str, nodes: &[DiskNode]) -> Result<()> {
        self.append_staging_iter(job_id, nodes.iter())
    }

    /// 从借用迭代器按批编码 staging，不克隆扫描节点。
    /// 写入、计数或清理指定代次的扫描暂存数据。
    /// 参数：job_id：本 fencing 代次的暂存命名空间；nodes：有序观测节点集合。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn append_staging_iter<'a>(
        &mut self,
        job_id: &str,
        nodes: impl Iterator<Item = &'a DiskNode>,
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO scan_staging (job_id, node_seq, node_json) VALUES (?1, ?2, ?3)",
            )?;
            let existing: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(node_seq), 0) FROM scan_staging WHERE job_id = ?1",
                [job_id],
                |row| row.get(0),
            )?;
            for (offset, node) in nodes.enumerate() {
                statement.execute(params![
                    job_id,
                    existing + offset as i64 + 1,
                    to_string(node)?
                ])?;
                let path = match &node.locator {
                    ResourceLocator::NativePath(path) | ResourceLocator::DocumentUri(path) => path,
                };
                transaction.execute(
                    "INSERT INTO scan_staging_search VALUES (?1, ?2, ?3, ?4)",
                    params![
                        job_id,
                        existing + offset as i64 + 1,
                        node.name.to_lowercase(),
                        path.to_lowercase()
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Number of staged nodes for one job.
    /// 写入、计数或清理指定代次的扫描暂存数据。
    /// 参数：job_id：本 fencing 代次的暂存命名空间。
    /// 返回：该命名空间的暂存节点数量。
    pub fn staging_node_count(&self, job_id: &str) -> Result<u64> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM scan_staging WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    /// Drops staging for a job without publishing anything.
    /// 写入、计数或清理指定代次的扫描暂存数据。
    /// 参数：job_id：本 fencing 代次的暂存命名空间。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn clear_staging(&mut self, job_id: &str) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM scan_staging WHERE job_id = ?1", [job_id])?;
        tx.execute(
            "DELETE FROM scan_staging_search WHERE job_id = ?1",
            [job_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 清理低于 `active_fence` 的扫描暂存代次。调用者须先在控制库认领该 fence，
    /// 或持久终结该 job 后以最后 fence + 1 清理；当前/更新代次及其他 job 均保留。
    /// 写入、计数或清理指定代次的扫描暂存数据。
    /// 参数：job_id：持久任务 ID；active_fence：已认领或持久终结后可安全清理的 fence 边界。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn clear_stale_job_staging(&mut self, job_id: &str, active_fence: u64) -> Result<()> {
        let prefix = format!("{job_id}:");
        let tx = self.connection.transaction()?;
        let stale: Vec<String> = {
            let mut statement = tx.prepare(
                "SELECT job_id FROM scan_staging WHERE substr(job_id, 1, length(?1)) = ?1
                 UNION SELECT job_id FROM scan_staging_search WHERE substr(job_id, 1, length(?1)) = ?1",
            )?;
            statement
                .query_map([&prefix], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|namespace| {
                    namespace
                        .strip_prefix(&prefix)
                        .and_then(|fence| fence.parse::<u64>().ok())
                        .is_some_and(|fence| fence < active_fence)
                })
                .collect()
        };
        for namespace in stale {
            tx.execute("DELETE FROM scan_staging WHERE job_id = ?1", [&namespace])?;
            tx.execute(
                "DELETE FROM scan_staging_search WHERE job_id = ?1",
                [&namespace],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
