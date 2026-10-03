//! 对有效采集批次逐批索引寻址，再合并关系键；不扫描同一实体的历史批次。
use crate::{Result, RevisionEvidenceReader, StoreError};
use diskgraph_core::{QueryReadBudget, Relation};
use rusqlite::params;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// 一个请求的有效批次 keyset 合并器，暂存每个方向的一条关系键。
/// 来源：DiskGraph EV-05 / D28；无 Java 对应对象。
pub(crate) struct RevisionEdgeCursor<'a, 's> {
    reader: &'a RevisionEvidenceReader<'s>,
    entity: &'a str,
    relation: Option<Relation>,
    cursors: Vec<(String, bool)>,
    heads: BinaryHeap<Reverse<(String, usize)>>,
}

impl<'a, 's> RevisionEdgeCursor<'a, 's> {
    /// 仅枚举有效运行，在拥有运行和关系键前计入请求账本。
    /// 参数：reader/entity 为固定资源，direction/relation/after 为分页条件，reads 为共享预算。
    /// 返回：有序合并器；无法完整枚举有效运行时拒绝输出不确定顺序的前缀。
    pub(crate) fn new(
        reader: &'a RevisionEvidenceReader<'s>,
        entity: &'a str,
        direction: Option<bool>,
        relation: Option<Relation>,
        after: Option<&str>,
        reads: &mut QueryReadBudget,
    ) -> Result<Self> {
        let mut result = Self {
            reader,
            entity,
            relation,
            cursors: Vec::new(),
            heads: BinaryHeap::new(),
        };
        let mut stmt=reader.store.connection.prepare("SELECT rr.run_id FROM revision_runs rr JOIN collector_runs cr ON cr.run_id=rr.run_id WHERE rr.revision_id=?1 AND rr.role='active' AND cr.snapshot_id=?2 ORDER BY rr.run_id")?;
        let mut rows = stmt.query(params![reader.revision_id, reader.snapshot_id])?;
        while let Some(row) = rows.next()? {
            let run = row.get_ref(0)?.as_str().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            for outgoing in [true, false] {
                if direction.is_some_and(|side| side != outgoing) {
                    continue;
                }
                // 同一运行的两个方向各自拥有一个键；固定容器成本也纳入暂存额度。
                if !reads.admit(0, 0, run.len().saturating_add(64)) {
                    return Err(StoreError::BudgetExceeded);
                }
                result.cursors.push((run.to_owned(), outgoing));
            }
        }
        for index in 0..result.cursors.len() {
            if let Some(key) = result.head(index, after, reads)? {
                result.heads.push(Reverse((key, index)));
            }
        }
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(result)
    }

    /// 借用最小关系键，不读取或解码额外关系 payload。
    /// 参数：无。返回：下一关系键，所有运行耗尽时为 None。
    pub(crate) fn peek(&self) -> Option<&str> {
        self.heads.peek().map(|Reverse((key, _))| key.as_str())
    }

    /// 推进所有命中当前键的运行，保证双向自环和重复成员只返回一次。
    /// 参数：reads 为共享预算。返回：推进成功或停止原因对应的错误。
    pub(crate) fn advance(&mut self, reads: &mut QueryReadBudget) -> Result<()> {
        let Some(Reverse((key, index))) = self.heads.pop() else {
            return Ok(());
        };
        self.advance_one(index, &key, reads)?;
        while self.peek() == Some(key.as_str()) {
            let Reverse((_, index)) = self.heads.pop().expect("peeked heap entry");
            self.advance_one(index, &key, reads)?;
        }
        Ok(())
    }

    fn advance_one(
        &mut self,
        index: usize,
        after: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<()> {
        if let Some(key) = self.head(index, Some(after), reads)? {
            self.heads.push(Reverse((key, index)));
        }
        Ok(())
    }

    fn head(
        &self,
        index: usize,
        after: Option<&str>,
        reads: &mut QueryReadBudget,
    ) -> Result<Option<String>> {
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let (run, outgoing) = &self.cursors[index];
        let side = if *outgoing {
            "source_entity_id"
        } else {
            "target_entity_id"
        };
        let comparison = if after.is_some() { ">" } else { ">=" };
        let sql = format!(
            "SELECT edge_id FROM relation_membership_adjacency WHERE snapshot_id=?1 AND run_id=?2 AND {side}=?3 AND edge_id {comparison} ?4 AND (?5 IS NULL OR relation=?5) ORDER BY edge_id LIMIT 1"
        );
        let mut stmt = self.reader.store.connection.prepare(&sql)?;
        let mut rows = stmt.query(params![
            self.reader.snapshot_id,
            run,
            self.entity,
            after.unwrap_or(""),
            self.relation.map(|value| value.wire_name())
        ])?;
        let row = rows.next()?;
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let Some(row) = row else {
            return Ok(None);
        };
        let key = row.get_ref(0)?.as_str().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        if !reads.admit(0, 0, key.len()) {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(Some(key.to_owned()))
    }
}
