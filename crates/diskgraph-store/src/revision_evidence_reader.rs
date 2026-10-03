//! 固定 revision 的证据读取入口；不暴露可退回 snapshot 查询的 Deref。
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{Entity, EvidenceRecord, QueryReadBudget, TruncationReason};
use rusqlite::{OptionalExtension, params};

/// 绑定已发布 revision 与真实文件快照的证据读取器。
/// 来源：DiskGraph EV-05 / D28；无 Java 对应对象。
pub struct RevisionEvidenceReader<'a> {
    pub(crate) store: &'a SqliteSnapshotStore,
    pub(crate) revision_id: String,
    pub(crate) snapshot_id: String,
}

impl SqliteSnapshotStore {
    /// 创建只观察指定 revision 批次的读取器。
    /// 参数：revision_id 为已发布标识；返回：固定实际 snapshot 的读取器，未知 revision 报错。
    pub fn revision_evidence(&self, revision_id: &str) -> Result<RevisionEvidenceReader<'_>> {
        let revision = self.revision(revision_id)?;
        Ok(RevisionEvidenceReader {
            store: self,
            revision_id: revision.revision_id,
            snapshot_id: revision.snapshot_id,
        })
    }
}

impl RevisionEvidenceReader<'_> {
    /// 返回已选择的 revision 标识。
    /// 参数：无。
    /// 返回：该读取器固定的标识。
    pub fn revision_id(&self) -> &str {
        &self.revision_id
    }

    /// 返回 revision 实际绑定的 snapshot 标识。
    /// 参数：无。
    /// 返回：该读取器固定的标识。
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    /// 检查旧成员归属是否存在不可确认条目。
    /// 参数：无；返回：true 要求重新采集，禁止把空结果解释为完整负证据。
    pub fn has_incomplete_membership(&self) -> Result<bool> {
        Ok(self.store.connection.query_row(
            "SELECT selection_sealed!=1 OR evidence_complete!=1 FROM graph_revisions WHERE revision_id=?1",
            [&self.revision_id],
            |row| row.get(0),
        )?)
    }

    /// 拒绝来源不明确的旧证据，避免把缺失结果当作完整负证据。
    /// 参数：无；返回：可确认来源时成功，否则要求管理员重采或重新索引。
    pub fn require_confirmed_membership(&self) -> Result<()> {
        if self.has_incomplete_membership()? {
            return Err(StoreError::InvalidGraph(
                "unconfirmed evidence membership; recollect or reindex this snapshot".into(),
            ));
        }
        Ok(())
    }

    /// 读取所选 active/dependency_only 批次的实体。
    /// 参数：entity_id 为精确标识；返回：实体或 None，保留旧 TEXT 契约。
    pub fn entity(&self, entity_id: &str) -> Result<Option<Entity>> {
        let sql = self.entity_sql();
        let json: Option<String> = self
            .store
            .connection
            .query_row(
                &sql,
                params![self.snapshot_id, entity_id, self.revision_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    /// 在拥有或解码实体字段前计入整个请求的预算。
    /// 参数：entity_id 为精确标识，reads 为共享账本；返回：实体、None 或预算/格式错误。
    pub fn entity_with_budget(
        &self,
        entity_id: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<Option<Entity>> {
        self.read_record(&self.entity_sql(), entity_id, reads, true)
    }

    /// 只解析所选 active/dependency_only 来源，依赖批次不产生断言。
    /// 参数：id 为证据 ID，reads 为共享账本；返回：证据、None 或预算/格式错误。
    pub fn evidence_record_with_budget(
        &self,
        id: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<Option<EvidenceRecord>> {
        self.read_record(Self::evidence_sql(), id, reads, false)
    }

    /// 内部兼容读取，保留严格 TEXT 类型。
    /// 参数：id 为证据 ID。
    /// 返回：所选来源记录或读取错误。
    pub(crate) fn evidence_record(&self, id: &str) -> Result<Option<EvidenceRecord>> {
        let json: Option<String> = self
            .store
            .connection
            .query_row(
                Self::evidence_sql(),
                params![self.snapshot_id, id, self.revision_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    fn entity_sql(&self) -> String {
        "SELECT e.entity_json FROM entities e WHERE e.snapshot_id=?1 AND e.entity_id=?2
         AND EXISTS(SELECT 1 FROM entity_run_memberships m
         JOIN revision_runs rr ON rr.run_id=m.run_id AND rr.revision_id=?3
         JOIN collector_runs cr ON cr.run_id=m.run_id AND cr.snapshot_id=m.snapshot_id
         WHERE m.snapshot_id=e.snapshot_id AND m.entity_id=e.entity_id
         AND rr.role IN ('active','dependency_only'))"
            .into()
    }

    fn evidence_sql() -> &'static str {
        "SELECT e.evidence_json FROM evidence_records e WHERE e.snapshot_id=?1 AND e.evidence_id=?2
         AND EXISTS(SELECT 1 FROM revision_runs rr
         JOIN collector_runs cr ON cr.run_id=rr.run_id AND cr.snapshot_id=e.snapshot_id
         WHERE rr.revision_id=?3 AND rr.run_id=e.run_id AND rr.role IN ('active','dependency_only'))"
    }

    fn read_record<T: serde::de::DeserializeOwned>(
        &self,
        sql: &str,
        id: &str,
        reads: &mut QueryReadBudget,
        entity: bool,
    ) -> Result<Option<T>> {
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let mut statement = self.store.connection.prepare(sql)?;
        let mut rows = statement.query(params![self.snapshot_id, id, self.revision_id])?;
        let row = rows.next()?;
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let Some(row) = row else {
            return Ok(None);
        };
        if !entity && reads.remaining_edges() == 0 {
            reads.stop(TruncationReason::EdgeLimit);
            return Err(StoreError::BudgetExceeded);
        }
        let value = row.get_ref(0)?;
        let json = value.as_bytes().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(0, value.data_type(), Box::new(error))
        })?;
        if !reads.admit(usize::from(entity), usize::from(!entity), json.len()) {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(Some(serde_json::from_slice(json)?))
    }
}
