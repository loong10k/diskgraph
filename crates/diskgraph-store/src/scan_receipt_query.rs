//! 有界读取扫描提交事实；损坏不得降级为尚未发布。来源：原生 Rust RT-01。
use crate::{Result, ScanPublicationReceipt, SqliteSnapshotStore, StoreError};
use rusqlite::{Connection, OptionalExtension, params};

impl SqliteSnapshotStore {
    /// 参数：实际任务 ID；返回：有限且严格核对的原回执或 None，回执保留不依赖历史是否已回收。
    pub fn scan_publication_receipt(&self, job_id: &str) -> Result<Option<ScanPublicationReceipt>> {
        let row: Option<(Option<String>,i64,Option<String>)> = self.connection.query_row(
            "SELECT CASE WHEN length(revision_id)<=256 THEN revision_id ELSE NULL END,publishing_fence,CASE WHEN length(CAST(receipt_json AS BLOB))<=16384 AND length(revision_id)<=256 THEN receipt_json ELSE NULL END FROM scan_publication_receipts WHERE job_id=?1",
            [job_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional()?;
        let Some((revision, fence, raw)) = row else {
            return Ok(None);
        };
        let invalid = || StoreError::InvalidGraph("corrupt scan publication receipt".into());
        let receipt: ScanPublicationReceipt =
            serde_json::from_str(&raw.ok_or_else(invalid)?).map_err(|_| invalid())?;
        receipt.validate()?;
        if receipt.job_id != job_id
            || receipt.revision_id != revision.ok_or_else(invalid)?
            || receipt.publishing_fence != fence as u64
        {
            return Err(invalid());
        }
        Ok(Some(receipt))
    }
}

/// 参数：当前图库事务及原回执；返回：唯一不可变账本写入，必须随正式 revision 同事务提交。
pub(crate) fn write(connection: &Connection, receipt: &ScanPublicationReceipt) -> Result<()> {
    receipt.validate()?;
    connection.execute("INSERT INTO scan_publication_receipts(job_id,revision_id,publishing_fence,receipt_json) VALUES(?1,?2,?3,?4)",
        params![receipt.job_id,receipt.revision_id,receipt.publishing_fence as i64,serde_json::to_string(receipt)?])?;
    Ok(())
}
