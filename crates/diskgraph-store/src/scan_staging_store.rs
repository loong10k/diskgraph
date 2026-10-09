//! 有界扫描暂存与失效 fencing 代次清理。

use crate::staging_node_encoding::kind_name;
use crate::{PreparedStagingNode, Result, SqliteSnapshotStore};
use diskgraph_core::{DiskNode, QualifiedLocator, WindowsFileObservation, WindowsObservationGap};
use rusqlite::params;
use std::borrow::Borrow;

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
        self.append_encoded_staging_iter(
            job_id,
            nodes.map(|node| (node, None, None, None, None)),
            || Ok(()),
        )
    }

    /// 按同一个暂存批次原子保存原生定位和节点自身时间，保留既有 fencing 命名空间。
    /// 参数：job_id 为当前任务代次；每项为节点、qualified locator、自身修改秒数。
    /// 返回：批次整体成功或全部回滚，编码损坏不会留下半个批次。
    pub fn append_staging_located_iter<'a>(
        &mut self,
        job_id: &str,
        nodes: impl Iterator<Item = (&'a DiskNode, &'a QualifiedLocator, Option<i64>)>,
    ) -> Result<()> {
        self.append_encoded_staging_iter(
            job_id,
            nodes.map(|(node, locator, modified)| (node, Some(locator), modified, None, None)),
            || Ok(()),
        )
    }

    /// 按批次原子保存节点定位、原生观测或固定缺失原因。
    /// 参数：job_id 为 fencing 命名空间；迭代项为节点、定位、自身时间、完整观测、缺失原因。
    /// 返回：全部写入成功或整个批次回滚；完整观测与缺失原因不能同时存在。
    pub fn append_staging_observed_iter<'a>(
        &mut self,
        job_id: &str,
        nodes: impl Iterator<
            Item = (
                &'a DiskNode,
                &'a QualifiedLocator,
                Option<i64>,
                Option<&'a WindowsFileObservation>,
                Option<WindowsObservationGap>,
            ),
        >,
    ) -> Result<()> {
        self.append_staging_observed_iter_checked(job_id, nodes, || Ok(()))
    }

    /// 在每项编码与实际批次 commit 前检查原授权时钟及取消。
    /// 参数：当前代次、原观测迭代器和只读不可变上下文的纯回调，不得重入控制库。
    /// 返回：整批提交或回滚；已有无回调接口保留可信内部兼容。
    pub fn append_staging_observed_iter_checked<'a>(
        &mut self,
        job_id: &str,
        nodes: impl Iterator<
            Item = (
                &'a DiskNode,
                &'a QualifiedLocator,
                Option<i64>,
                Option<&'a WindowsFileObservation>,
                Option<WindowsObservationGap>,
            ),
        >,
        check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        self.append_encoded_staging_iter(
            job_id,
            nodes.map(|(node, locator, modified, observation, gap)| {
                (node, Some(locator), modified, observation, gap)
            }),
            check,
        )
    }

    fn append_encoded_staging_iter<'a>(
        &mut self,
        job_id: &str,
        nodes: impl Iterator<
            Item = (
                &'a DiskNode,
                Option<&'a QualifiedLocator>,
                Option<i64>,
                Option<&'a WindowsFileObservation>,
                Option<WindowsObservationGap>,
            ),
        >,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        self.append_prepared_results(
            job_id,
            nodes.map(|(node, locator, modified, observation, gap)| {
                PreparedStagingNode::new(node, locator.cloned(), modified, observation, gap)
            }),
            &mut check,
        )
    }

    /// 写入已按实际编码成本准入的不可变批次，不重新编码或读取源路径。
    /// 参数：job_id 为原 fencing 命名空间，nodes 只借用当前批次，check 沿原 clock/cancel/fence。
    /// 返回：全批提交或回滚，节点和搜索字段共用同一事务；调用方负责预算准入与实时授权。
    pub fn append_prepared_staging_iter_checked<'a>(
        &mut self,
        job_id: &str,
        nodes: impl Iterator<Item = &'a PreparedStagingNode>,
        check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        self.append_prepared_results(job_id, nodes.map(Ok), check)
    }

    /// 延迟批次的自动 checkpoint，供调用方在释放原控制 fence 后维护。
    /// 参数：当前 fencing 命名空间、已编码节点、累计维护责任与原准入检查；返回：提交/恢复结果。
    /// 调用方必须在下一批次之前处理真实提交产生的维护请求；旧可信入口保持兼容。
    pub fn append_prepared_staging_iter_checked_deferred<'a>(
        &mut self,
        job_id: &str,
        nodes: impl Iterator<Item = &'a PreparedStagingNode>,
        checkpoint_due: &mut bool,
        check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        let checkpoint =
            crate::staging_checkpoint_guard::StagingCheckpointGuard::suspend(&self.connection)?;
        self.append_prepared_results(job_id, nodes.map(Ok), check)?;
        checkpoint.finish(checkpoint_due)
    }

    /// 在原控制 fence 已释放之后执行非阻塞读者的 WAL 维护。
    /// 参数：无，使用原图库写连接；返回：维护调用成功或 SQLite 错误。
    /// PASSIVE 不强迫现有读事务结束，剩余 WAL 继续由实际容量门禁计费。
    pub fn checkpoint_after_staging(&self) -> Result<()> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE)")?;
        Ok(())
    }

    fn append_prepared_results<S: Borrow<PreparedStagingNode>>(
        &self,
        job_id: &str,
        mut nodes: impl Iterator<Item = Result<S>>,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        check()?;
        // 外层公开入口仍要求 &mut self；共享借用使原 checkpoint 守卫可跨越事务。
        // SQLite 仍拒绝嵌套 BEGIN，不绕过实际事务检查。
        let transaction = self.connection.unchecked_transaction()?;
        check()?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO scan_staging (job_id,node_seq,node_json,native_locator_kind,native_locator_encoding,native_locator_raw,self_modified_unix_seconds,native_observation_format,native_observation_raw,native_observation_gap) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?;
            // 每批编译一次搜索写入，逐节点仅绑定值；仍与节点写入共用同一事务。
            let mut search_statement =
                transaction.prepare("INSERT INTO scan_staging_search VALUES (?1,?2,?3,?4)")?;
            let existing: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(node_seq),0) FROM scan_staging WHERE job_id=?1",
                [job_id],
                |row| row.get(0),
            )?;
            let mut offset = 0i64;
            loop {
                // 先检查，再推进可能执行编码的旧接口迭代器；外部纯回调不得重入此库。
                check()?;
                let Some(prepared) = nodes.next() else { break };
                let prepared = prepared?;
                let prepared = prepared.borrow();
                let encoded = &prepared.encoded;
                let locator = prepared.locator.as_ref();
                check()?;
                let sequence = existing
                    .checked_add(offset)
                    .and_then(|value| value.checked_add(1))
                    .ok_or(crate::StoreError::IntegerOverflow)?;
                statement.execute(params![
                    job_id,
                    sequence,
                    encoded.json,
                    locator.map(|value| kind_name(value.kind())),
                    locator.map(|value| value.encoding().wire_name()),
                    locator.map(|value| value.raw_bytes()),
                    prepared.modified,
                    encoded
                        .observation_raw
                        .as_ref()
                        .map(|_| WindowsFileObservation::FORMAT_LABEL),
                    encoded.observation_raw.as_ref().map(|raw| raw.as_slice()),
                    prepared.gap.map(|value| value.code())
                ])?;
                search_statement.execute(params![
                    job_id,
                    sequence,
                    encoded.name_fold,
                    encoded.path_fold
                ])?;
                offset = offset
                    .checked_add(1)
                    .ok_or(crate::StoreError::IntegerOverflow)?;
            }
        }
        check()?;
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
            "DELETE FROM scan_staging_unix_observations WHERE job_id = ?1",
            [job_id],
        )?;
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
                 UNION SELECT job_id FROM scan_staging_search WHERE substr(job_id, 1, length(?1)) = ?1
                 UNION SELECT job_id FROM scan_staging_unix_observations WHERE substr(job_id, 1, length(?1)) = ?1",
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
                "DELETE FROM scan_staging_unix_observations WHERE job_id = ?1",
                [&namespace],
            )?;
            tx.execute(
                "DELETE FROM scan_staging_search WHERE job_id = ?1",
                [&namespace],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "staging_checkpoint_tests.rs"]
mod checkpoint_tests;
