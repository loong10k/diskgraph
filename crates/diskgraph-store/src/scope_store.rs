//! 稳定 server 身份和 scope 注册、查询、撤销。

use rusqlite::OptionalExtension;

use crate::control_codec::{locator_kind_tag, parse_locator_kind};
use crate::{ControlStore, Result, ScopeRecord, StoreError};
use diskgraph_core::{Locator, ScopeId, ServerId};
use rusqlite::params;
use uuid::Uuid;

impl ControlStore {
    /// Mints the server identity on first use and returns the stored one after.
    /// 持久管理稳定服务器身份及范围注册、读取和撤销。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：`Result<ServerId>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn ensure_server(&mut self) -> Result<ServerId> {
        let existing: Option<String> = self
            .connection
            .query_row("SELECT server_id FROM server WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(server_id) = existing {
            let server_id = ServerId::new(server_id).map_err(|error| {
                StoreError::InvalidGraph(format!("stored server id invalid: {error}"))
            })?;
            return Ok(server_id);
        }
        let server_id = ServerId::new(format!("srv-{}", Uuid::new_v4()))
            .map_err(|error| StoreError::InvalidGraph(error.to_string()))?;
        self.connection.execute(
            "INSERT INTO server (id, server_id, created_at_unix_ms) VALUES (1, ?1, ?2)",
            params![server_id.as_str(), Self::now_ms() as i64],
        )?;
        Ok(server_id)
    }

    /// Registers (or idempotently returns) a scope for this lossless root.
    /// A revoked scope with the same root blocks re-registration until a
    /// deliberate policy change (SC-04).
    /// 持久管理稳定服务器身份及范围注册、读取和撤销。
    /// 参数：root：无损根定位或根过滤条件；volume_id：可选卷身份。
    /// 返回：`Result<ScopeId>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn register_scope(&mut self, root: &Locator, volume_id: Option<&str>) -> Result<ScopeId> {
        let existing: Option<(String, i64)> = self
            .connection
            .query_row(
                "SELECT scope_id, revoked FROM scopes
                 WHERE root_kind = ?1 AND root_raw_b64 = ?2",
                params![locator_kind_tag(root.kind), root.raw_b64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((scope_id, revoked)) = existing {
            if revoked != 0 {
                return Err(StoreError::Conflict(format!(
                    "scope {scope_id} was revoked; re-registration needs an explicit decision"
                )));
            }
            let scope_id = ScopeId::new(scope_id)
                .map_err(|error| StoreError::InvalidGraph(error.to_string()))?;
            return Ok(scope_id);
        }
        let scope_id = ScopeId::new(format!("scope-{}", Uuid::new_v4()))
            .map_err(|error| StoreError::InvalidGraph(error.to_string()))?;
        self.connection.execute(
            "INSERT INTO scopes (scope_id, root_kind, root_raw_b64, root_display, volume_id, created_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                scope_id.as_str(),
                locator_kind_tag(root.kind),
                root.raw_b64,
                root.display,
                volume_id,
                Self::now_ms() as i64,
            ],
        )?;
        Ok(scope_id)
    }

    /// Loads one scope; missing scopes are a hard error, revoked ones carry the flag.
    /// 持久管理稳定服务器身份及范围注册、读取和撤销。
    /// 参数：scope_id：实际所属范围 ID。
    /// 返回：`Result<ScopeRecord>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn scope(&self, scope_id: &ScopeId) -> Result<ScopeRecord> {
        let record = self
            .connection
            .query_row(
                "SELECT root_kind, root_raw_b64, root_display, volume_id, created_at_unix_ms, revoked
                 FROM scopes WHERE scope_id = ?1",
                [scope_id.as_str()],
                |row| {
                    Ok(ScopeRecord {
                        scope_id: scope_id.clone(),
                        root: Locator {
                            kind: parse_locator_kind(&row.get::<_, String>(0)?),
                            raw_b64: row.get(1)?,
                            display: row.get(2)?,
                        },
                        volume_id: row.get(3)?,
                        created_at_unix_ms: row.get::<_, i64>(4)?.try_into().unwrap_or(0),
                        revoked: row.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::ScopeNotFound(scope_id.to_string()))?;
        Ok(record)
    }

    /// All registered scopes, oldest first.
    /// 持久管理稳定服务器身份及范围注册、读取和撤销。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：`Result<Vec<ScopeRecord>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn list_scopes(&self) -> Result<Vec<ScopeRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT scope_id, root_kind, root_raw_b64, root_display, volume_id, created_at_unix_ms, revoked
             FROM scopes ORDER BY created_at_unix_ms ASC, scope_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ScopeRecord {
                scope_id: ScopeId::new(row.get::<_, String>(0)?)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
                root: Locator {
                    kind: parse_locator_kind(&row.get::<_, String>(1)?),
                    raw_b64: row.get(2)?,
                    display: row.get(3)?,
                },
                volume_id: row.get(4)?,
                created_at_unix_ms: row.get::<_, i64>(5)?.try_into().unwrap_or(0),
                revoked: row.get::<_, i64>(6)? != 0,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Revokes a scope: lookups and jobs must treat it as gone (SC-04).
    /// Idempotent; never deletes files, snapshots, or recovery records.
    /// 持久管理稳定服务器身份及范围注册、读取和撤销。
    /// 参数：scope_id：实际所属范围 ID。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn revoke_scope(&mut self, scope_id: &ScopeId) -> Result<()> {
        self.scope(scope_id)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE scopes SET revoked = 1 WHERE scope_id = ?1",
            [scope_id.as_str()],
        )?;
        // 仅排队 Git 在本次撤销中直接终结；运行任务仍由原 fence 结算实际取消。
        tx.execute(
            "INSERT INTO git_job_failures(job_id,phase,code)
             SELECT job_id,'admission','cancelled' FROM jobs
             WHERE scope_id=?1 AND kind='git_evidence' AND state='queued'",
            [scope_id.as_str()],
        )?;
        crate::process_job_failure_store::record_queued_cancel(&tx, "scope_id", scope_id.as_str())?;
        tx.execute(
            "UPDATE jobs SET state = 'cancelled' WHERE scope_id = ?1 AND state = 'queued'",
            [scope_id.as_str()],
        )?;
        tx.execute(
            "UPDATE jobs SET cancel_requested = 1 WHERE scope_id = ?1 AND state = 'running'",
            [scope_id.as_str()],
        )?;
        tx.commit()?;
        self.reap_unclaimable_jobs()?;
        Ok(())
    }
}
