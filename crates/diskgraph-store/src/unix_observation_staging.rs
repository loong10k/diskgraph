//! 扫描 Unix 强身份旁表的真实 checked 批次；来源：原生 Rust D42 / FS-02。
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{UnixFileObservation, UnixObservationGap};
use rusqlite::{Connection, params};
impl SqliteSnapshotStore {
    /// 参数：已有 staging namespace、每项节点/观察/缺失码和纯 clock/cancel/fence 检查。
    /// 返回：全部准入并提交，未知/重复/跨平台节点或任一末检错误整体回滚。
    /// 本可信接口仅接受扫描实际捕获；不打开源对象，也不由执行现场补旧索引身份。
    pub fn append_staging_unix_observations_checked<'a>(
        &mut self,
        job: &str,
        items: impl Iterator<
            Item = (
                u64,
                Option<&'a UnixFileObservation>,
                Option<UnixObservationGap>,
            ),
        >,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        check()?;
        let tx = self.connection.transaction()?;
        check()?;
        for (node, observation, gap) in items {
            check()?;
            let id = crate::node_codec::as_i64(node)?;
            let exists:bool=tx.query_row("SELECT COUNT(*)=1 FROM scan_staging WHERE job_id=?1 AND json_extract(node_json,'$.id')=?2 AND native_locator_kind='native_path' AND native_locator_encoding='unix_bytes' AND json_extract(node_json,'$.kind')='file' AND json_extract(node_json,'$.read_error')=0",params![job,id],|r|r.get(0))?;
            if !exists || observation.is_some() == gap.is_some() {
                return Err(StoreError::InvalidGraph(
                    "invalid Unix observation staging target".into(),
                ));
            }
            let encoded =
                crate::staging_unix_observation_encoding::StagingUnixObservationEncoding::encode(
                    observation,
                    gap,
                )?;
            tx.execute(
                "INSERT INTO scan_staging_unix_observations VALUES(?1,?2,?3,?4,14)",
                params![job, id, encoded.raw, encoded.gap],
            )?;
            check()?;
        }
        check()?;
        tx.commit()?;
        Ok(())
    }
}
/// 参数：实际扫描发布事务、namespace/snapshot；返回：先验证再复制同一已完成 staging 观察。
pub(crate) fn publish(connection: &Connection, job: &str, snapshot: &str) -> Result<()> {
    let mismatched:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM scan_staging_unix_observations u WHERE u.job_id=?1 AND NOT EXISTS(SELECT 1 FROM nodes n WHERE n.snapshot_id=?2 AND n.id=u.node_id AND n.native_locator_kind='native_path' AND n.native_locator_encoding='unix_bytes' AND COALESCE(n.kind,json_extract(NULLIF(n.node_json,''),'$.kind'))='file'))",params![job,snapshot],|r|r.get(0))?;
    if mismatched {
        return Err(StoreError::InvalidGraph(
            "Unix observation does not match published scan".into(),
        ));
    }
    connection.execute("INSERT INTO node_unix_observations SELECT ?2,node_id,observation_raw,gap,writer_generation FROM scan_staging_unix_observations WHERE job_id=?1",params![job,snapshot])?;
    connection.execute(
        "DELETE FROM scan_staging_unix_observations WHERE job_id=?1",
        [job],
    )?;
    Ok(())
}
