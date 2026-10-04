//! 读取持久图库回执，不从当前 latest 或新 fence 推算结果；来源：原生 Rust RT-01。
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::ProcessJobPublicationReceipt;
use rusqlite::Connection;

impl SqliteSnapshotStore {
    /// 参数：真实任务 ID；返回：16KiB 原始准入后验证的提交事实或 None，损坏绝不当作未发布。
    /// 回执不级联历史删除；已回收的结果不可因此重新发布。
    pub fn process_job_publication_receipt(
        &self,
        job_id: &str,
    ) -> Result<Option<ProcessJobPublicationReceipt>> {
        read(&self.connection, job_id)
    }
}
/// 参数：当前图库事务/连接和任务 ID；返回：严格回执或明确错误。
pub(crate) fn read(
    connection: &Connection,
    job_id: &str,
) -> Result<Option<ProcessJobPublicationReceipt>> {
    let mut statement=connection.prepare("SELECT schema_version,input_sha256,server_id,scope_id,snapshot_id,
        revision_id,run_id,node_id,CASE WHEN length(CAST(receipt_json AS BLOB))<=16384 THEN receipt_json ELSE NULL END,
        writer_generation FROM process_job_publication_receipts WHERE job_id=?1")?;
    let mut rows = statement.query([job_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let invalid = || StoreError::InvalidGraph("invalid or oversized publication receipt".into());
    if row.get::<_, i64>(0)? != 1 || row.get::<_, i64>(9)? != 14 {
        return Err(invalid());
    }
    // 冗余列同样来自磁盘，必须先借用计量全部字段，再解码合法的小回执。
    let fields =
        [1, 2, 3, 4, 5, 6, 8].map(|column| row.get_ref(column)?.as_str().map_err(|_| invalid()));
    let [digest, server, scope, snapshot, revision, run, raw] = fields;
    let (digest, server, scope, snapshot, revision, run, raw) =
        (digest?, server?, scope?, snapshot?, revision?, run?, raw?);
    let bytes = [digest, server, scope, snapshot, revision, run, raw]
        .iter()
        .try_fold(0_usize, |total, field| total.checked_add(field.len()))
        .ok_or_else(invalid)?;
    if bytes > 16384 {
        return Err(invalid());
    }
    let receipt: ProcessJobPublicationReceipt = serde_json::from_str(raw).map_err(|_| invalid())?;
    if receipt.job_id() != job_id
        || receipt.input_sha256() != digest
        || receipt.server_id().as_str() != server
        || receipt.scope_id().as_str() != scope
        || receipt.snapshot_id() != snapshot
        || receipt.revision_id() != revision
        || receipt.run_id() != run
        || receipt.input().node_id() != row.get::<_, i64>(7)? as u64
    {
        return Err(invalid());
    }
    Ok(Some(receipt))
}
