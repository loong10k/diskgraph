//! 有界扫描暂存与失效 fencing 代次清理。

use crate::staging_node_encoding::{StagingNodeEncoding, kind_name};
use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::{DiskNode, QualifiedLocator, WindowsFileObservation, WindowsObservationGap};
use rusqlite::params;

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
        check()?;
        let transaction = self.connection.transaction()?;
        check()?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO scan_staging (job_id,node_seq,node_json,native_locator_kind,native_locator_encoding,native_locator_raw,self_modified_unix_seconds,native_observation_format,native_observation_raw,native_observation_gap) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?;
            let existing: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(node_seq),0) FROM scan_staging WHERE job_id=?1",
                [job_id],
                |row| row.get(0),
            )?;
            for (offset, (node, locator, modified, observation, gap)) in nodes.enumerate() {
                check()?;
                let encoded = StagingNodeEncoding::encode_observed(
                    node,
                    locator,
                    modified,
                    observation,
                    gap,
                )?;
                check()?;
                let sequence = existing
                    .checked_add(
                        i64::try_from(offset).map_err(|_| crate::StoreError::IntegerOverflow)?,
                    )
                    .and_then(|value| value.checked_add(1))
                    .ok_or(crate::StoreError::IntegerOverflow)?;
                statement.execute(params![
                    job_id,
                    sequence,
                    encoded.json,
                    locator.map(|value| kind_name(value.kind())),
                    locator.map(|value| value.encoding().wire_name()),
                    locator.map(|value| value.raw_bytes()),
                    modified,
                    observation.map(|_| WindowsFileObservation::FORMAT_LABEL),
                    encoded.observation_raw.as_ref().map(|raw| raw.as_slice()),
                    gap.map(|value| value.code())
                ])?;
                transaction.execute(
                    "INSERT INTO scan_staging_search VALUES (?1,?2,?3,?4)",
                    params![job_id, sequence, encoded.name_fold, encoded.path_fold],
                )?;
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
