//! 固定 revision 目标的共享原始读取预算；来源：原生 Rust EC-02 / Q09，不读取完整快照。

use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::QueryReadBudget;
use rusqlite::types::ValueRef;

impl SqliteSnapshotStore {
    /// 先借用并准入真实归属字段，供授权前准备使用；来源：原生 Rust Q-08 / D41。
    /// 参数：revision_id 为固定历史，reads 为整次共享账本；返回：可选实际 server/scope。
    pub fn revision_ownership_with_budget(
        &self,
        revision_id: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<Option<(String, String)>> {
        check(reads)?;
        let mut statement = self
            .connection
            .prepare("SELECT server_id,scope_id FROM revision_ownership WHERE revision_id=?1")?;
        let mut rows = statement.query([revision_id])?;
        let row = rows.next()?;
        check(reads)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let server = text(row.get_ref(0)?)?;
        let scope = text(row.get_ref(1)?)?;
        let bytes = server
            .len()
            .checked_add(scope.len())
            .ok_or(StoreError::BudgetExceeded)?;
        if !reads.admit(0, 0, bytes) {
            return Err(StoreError::BudgetExceeded);
        }
        let owner = (server.to_owned(), scope.to_owned());
        check(reads)?;
        Ok(Some(owner))
    }

    /// 在已授权后准入必要 snapshot 标识，不重读或重复拥有归属；来源：原生 Rust Q-08 / D41。
    /// 参数：revision_id 为固定历史，reads 沿用归属及后续节点账本；返回：必要快照标识或预算错误。
    pub fn revision_snapshot_with_budget(
        &self,
        revision_id: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<String> {
        check(reads)?;
        let mut statement = self
            .connection
            .prepare("SELECT snapshot_id FROM graph_revisions WHERE revision_id=?1")?;
        let mut rows = statement.query([revision_id])?;
        let row = rows.next()?;
        check(reads)?;
        let row = row.ok_or_else(|| StoreError::RevisionNotFound(revision_id.into()))?;
        let snapshot = text(row.get_ref(0)?)?;
        if !reads.admit(0, 0, snapshot.len()) {
            return Err(StoreError::BudgetExceeded);
        }
        let snapshot = snapshot.to_owned();
        check(reads)?;
        Ok(snapshot)
    }

    /// 在拥有 snapshot/server/scope 前，计量实际三个字段并复用请求原期限。
    /// 参数：revision_id 是固定版本，reads 是后续节点与定位读取共用的账本。
    /// 返回：快照 ID 和可选真实归属；双 NULL 为未绑定旧版本，部分归属或损坏明确拒绝。
    pub fn revision_target_with_budget(
        &self,
        revision_id: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<(String, Option<(String, String)>)> {
        check(reads)?;
        let mut statement = self.connection.prepare(
            "SELECT r.snapshot_id,o.server_id,o.scope_id FROM graph_revisions r
             LEFT JOIN revision_ownership o ON o.revision_id=r.revision_id WHERE r.revision_id=?1",
        )?;
        let mut rows = statement.query([revision_id])?;
        let row = rows.next()?;
        check(reads)?;
        let row = row.ok_or_else(|| StoreError::RevisionNotFound(revision_id.into()))?;
        let snapshot = text(row.get_ref(0)?)?;
        let owner = match (row.get_ref(1)?, row.get_ref(2)?) {
            (ValueRef::Null, ValueRef::Null) => None,
            (server @ ValueRef::Text(_), scope @ ValueRef::Text(_)) => {
                Some((text(server)?, text(scope)?))
            }
            _ => return Err(invalid()),
        };
        let raw_bytes = owner
            .map_or(Some(snapshot.len()), |(server, scope)| {
                snapshot
                    .len()
                    .checked_add(server.len())?
                    .checked_add(scope.len())
            })
            .ok_or(StoreError::BudgetExceeded)?;
        if !reads.admit(0, 0, raw_bytes) {
            return Err(StoreError::BudgetExceeded);
        }
        let result = (
            snapshot.to_owned(),
            owner.map(|(server, scope)| (server.to_owned(), scope.to_owned())),
        );
        check(reads)?;
        Ok(result)
    }
}

fn text<'a>(value: ValueRef<'a>) -> Result<&'a str> {
    value.as_str().map_err(|_| invalid())
}
fn invalid() -> StoreError {
    StoreError::InvalidGraph("invalid revision target columns".into())
}
fn check(reads: &mut QueryReadBudget) -> Result<()> {
    if reads.check() {
        Ok(())
    } else {
        Err(StoreError::BudgetExceeded)
    }
}
