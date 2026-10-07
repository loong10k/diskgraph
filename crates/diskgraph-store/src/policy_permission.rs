//! 管理员单项授权读取，逐行借用文本以保留损坏数据拒绝语义。
use crate::{ControlStore, Result};
use diskgraph_core::{Permission, PrincipalId, ScopeId};
use rusqlite::types::FromSqlError;

impl ControlStore {
    /// 在同一 SQLite 观察中检查精确授权，不重建完整授权器。
    /// 参数：principal、permission、scope 为精确授权身份，版本来自当前策略。
    /// 返回：未发布为 None；撤销、无效版本或没有精确匹配为 Some(false)。
    /// 不读取 scope 撤销，仅供既有管理员伪 scope；普通范围使用 live_permission。
    /// 为保留无关损坏记录拒绝仍逐行检查全部 grant，读取量和 CPU 随授权数量增长。
    pub fn policy_permission(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<Option<bool>> {
        let mut statement = self.connection.prepare(
            "SELECT p.version,p.revoked,g.rowid,g.principal_id,g.permission,g.scope_id,g.policy_version
             FROM policy p LEFT JOIN grants g ON p.version>0 AND p.revoked=0 WHERE p.id=1",
        )?;
        let mut rows = statement.query([])?;
        let mut published = false;
        let mut allowed = false;
        let wire = permission.wire_name();
        while let Some(row) = rows.next()? {
            published = true;
            let version: i64 = row.get(0)?;
            let revoked: i64 = row.get(1)?;
            if version <= 0 || revoked != 0 {
                return Ok(Some(false));
            }
            if row.get::<_, Option<i64>>(2)?.is_none() {
                continue;
            }
            // 借用 SQLite 行缓冲区验证全部文本的类型与 UTF-8；不分配 Grant/String 集合。
            let actor = borrowed_text(row, 3)?;
            let capability = borrowed_text(row, 4)?;
            let owner = borrowed_text(row, 5)?;
            let grant_version: i64 = row.get(6)?;
            allowed |= actor == principal.as_str()
                && capability == wire
                && owner == scope.as_str()
                && grant_version == version;
        }
        Ok(published.then_some(allowed))
    }
}

// 无效类型与 UTF-8 保留存储解码错误，不把无关损坏记录当成授权成功。
fn borrowed_text<'a>(row: &'a rusqlite::Row<'_>, index: usize) -> rusqlite::Result<&'a str> {
    let value = row.get_ref(index)?;
    value.as_str().map_err(|error| match error {
        FromSqlError::InvalidType => rusqlite::Error::InvalidColumnType(
            index,
            row.as_ref()
                .column_name(index)
                .unwrap_or("grant")
                .to_owned(),
            value.data_type(),
        ),
        other => {
            rusqlite::Error::FromSqlConversionFailure(index, value.data_type(), Box::new(other))
        }
    })
}
