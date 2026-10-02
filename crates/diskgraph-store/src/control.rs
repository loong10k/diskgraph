//! Control-plane storage (P1 tasks 2.1/2.2/2.8, specs SC-01 / SC-04 / ST-05 /
//! RT-01): server identity, registered scopes, policy grants, and durable jobs
//! with owner fencing. This database must be backed up; it never rides along
//! with rebuildable graph indexes (design D5).

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use diskgraph_core::{
    FileActionKind, Grant, Locator, LocatorKind, Permission, PolicyAuthorizer, PrincipalId,
    ScopeId, ServerId,
};

use crate::{Result, StoreError};

/// A registered observation scope with its lossless root locator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScopeRecord {
    pub scope_id: ScopeId,
    pub root: Locator,
    pub volume_id: Option<String>,
    pub created_at_unix_ms: u64,
    pub revoked: bool,
}

/// What kind of durable work a job represents.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Index,
    Sync,
}

impl JobKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Sync => "sync",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "index" => Some(Self::Index),
            "sync" => Some(Self::Sync),
            _ => None,
        }
    }
}

/// Durable job lifecycle; `latest` never advances for Failed or Cancelled jobs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// One durable job with its fencing owner token.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: String,
    pub scope_id: ScopeId,
    pub kind: JobKind,
    pub state: JobState,
    pub created_at_unix_ms: u64,
    pub heartbeat_unix_ms: u64,
    pub owner: String,
    /// 每次重新认领递增，旧 owner 的写入必须携带并验证此值。
    #[serde(default)]
    pub fencing_token: u64,
    /// 当前租约结束时间；默认 30 秒。
    #[serde(default)]
    pub lease_expires_unix_ms: u64,
    /// The principal who requested the job; per-principal quotas count on it.
    pub principal: PrincipalId,
}

/// Control-plane database: identity, scopes, policy, and jobs.
pub struct ControlStore {
    connection: Connection,
}

impl ControlStore {
    /// Opens (creating if needed) a control database. On Unix the file is
    /// restricted to owner-only permissions.
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if (1..5).contains(&version) {
            let backup_dir = path
                .parent()
                .unwrap_or(Path::new("."))
                .join("migration_backups");
            std::fs::create_dir_all(&backup_dir)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    StoreError::InvalidGraph("unrepresentable control database path".into())
                })?;
            connection.backup(
                rusqlite::MAIN_DB,
                backup_dir.join(format!(
                    "{name}.pre-v{}.bak",
                    if version < 4 { 4 } else { 5 }
                )),
                None,
            )?;
        }
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path)?.permissions();
            permissions.set_mode(0o600);
            std::fs::set_permissions(path, permissions)?;
        }
        Self::initialize(connection)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self> {
        // `version` is the schema version we migrate FROM; it advances as each
        // migration runs, so a fresh database walks exactly the same path an
        // older file would and never skips a step.
        let mut version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if !(0..=5).contains(&version) {
            return Err(StoreError::UnsupportedSchema(version));
        }
        if version == 0 {
            connection.execute_batch(
                "PRAGMA foreign_keys = ON;
                 BEGIN IMMEDIATE;
                 CREATE TABLE server (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     server_id TEXT NOT NULL,
                     created_at_unix_ms INTEGER NOT NULL
                 );
                 CREATE TABLE scopes (
                     scope_id TEXT PRIMARY KEY,
                     root_kind TEXT NOT NULL,
                     root_raw_b64 TEXT NOT NULL,
                     root_display TEXT NOT NULL,
                     volume_id TEXT,
                     created_at_unix_ms INTEGER NOT NULL,
                     revoked INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE UNIQUE INDEX scopes_by_root
                     ON scopes (root_kind, root_raw_b64);
                 CREATE TABLE policy (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     version INTEGER NOT NULL,
                     revoked INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE grants (
                     principal_id TEXT NOT NULL,
                     permission TEXT NOT NULL,
                     scope_id TEXT NOT NULL,
                     policy_version INTEGER NOT NULL,
                     PRIMARY KEY (principal_id, permission, scope_id, policy_version)
                 );
                 CREATE TABLE jobs (
                     job_id TEXT PRIMARY KEY,
                     scope_id TEXT NOT NULL REFERENCES scopes (scope_id),
                     kind TEXT NOT NULL,
                     state TEXT NOT NULL,
                     created_at_unix_ms INTEGER NOT NULL,
                     heartbeat_unix_ms INTEGER NOT NULL,
                     owner TEXT NOT NULL DEFAULT ''
                 );
                 CREATE INDEX jobs_by_scope_state
                     ON jobs (scope_id, state, created_at_unix_ms DESC);
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        if version < 2 {
            Self::migrate_v1_to_v2(&connection)?;
            version = 2;
        }
        if version < 3 {
            Self::migrate_v2_to_v3(&connection)?;
            version = 3;
        }
        if version < 4 {
            connection.execute_batch("BEGIN IMMEDIATE; ALTER TABLE jobs ADD COLUMN fencing_token INTEGER NOT NULL DEFAULT 0; ALTER TABLE jobs ADD COLUMN lease_expires_unix_ms INTEGER NOT NULL DEFAULT 0; UPDATE jobs SET lease_expires_unix_ms = heartbeat_unix_ms + 30000 WHERE state = 'running'; PRAGMA user_version = 4; COMMIT;")?;
            version = 4;
        }
        if version < 5 {
            connection.execute_batch("BEGIN IMMEDIATE; ALTER TABLE jobs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0; PRAGMA user_version = 5; COMMIT;")?;
            version = 5;
        }
        debug_assert_eq!(version, 5, "every control migration must have run");
        Ok(Self { connection })
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_millis()
            .try_into()
            .expect("timestamp beyond u64")
    }

    /// Adds the job creator to the control schema (P4 task 5.5). Existing rows
    /// keep an empty principal, which never matches a real one.
    fn migrate_v1_to_v2(connection: &Connection) -> Result<()> {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             ALTER TABLE jobs ADD COLUMN principal TEXT NOT NULL DEFAULT '';
             CREATE INDEX IF NOT EXISTS jobs_by_principal_state
                 ON jobs (principal, state);
             PRAGMA user_version = 2;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Adds the execution control plane (P5 tasks 6.1-6.5): immutable plans,
    /// digest-bound approvals, operations with per-principal idempotency
    /// keys, per-item intents, and durable recovery records. A fresh database
    /// walks this same migration, so no path skips it.
    fn migrate_v2_to_v3(connection: &Connection) -> Result<()> {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE plans (
                 plan_id TEXT PRIMARY KEY,
                 scope_id TEXT NOT NULL REFERENCES scopes (scope_id),
                 principal TEXT NOT NULL,
                 action TEXT NOT NULL,
                 plan_json TEXT NOT NULL,
                 digest TEXT NOT NULL,
                 created_at_unix_ms INTEGER NOT NULL,
                 expires_at_unix_ms INTEGER NOT NULL,
                 state TEXT NOT NULL
             );
             CREATE INDEX plans_by_scope ON plans (scope_id, state);
             CREATE TABLE approvals (
                 approval_ref TEXT PRIMARY KEY,
                 plan_id TEXT NOT NULL REFERENCES plans (plan_id),
                 plan_digest TEXT NOT NULL,
                 principal TEXT NOT NULL,
                 action TEXT NOT NULL,
                 issued_by TEXT NOT NULL,
                 issued_at_unix_ms INTEGER NOT NULL,
                 expires_at_unix_ms INTEGER NOT NULL,
                 revoked INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE operations (
                 operation_id TEXT PRIMARY KEY,
                 plan_id TEXT NOT NULL REFERENCES plans (plan_id),
                 scope_id TEXT NOT NULL,
                 principal TEXT NOT NULL,
                 idempotency_key TEXT NOT NULL,
                 request_digest TEXT NOT NULL,
                 state TEXT NOT NULL,
                 created_at_unix_ms INTEGER NOT NULL,
                 updated_at_unix_ms INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX operations_by_idempotency
                 ON operations (principal, idempotency_key);
             CREATE TABLE operation_items (
                 operation_id TEXT NOT NULL REFERENCES operations (operation_id),
                 item_index INTEGER NOT NULL,
                 intent_state TEXT NOT NULL,
                 result_state TEXT NOT NULL,
                 detail TEXT NOT NULL DEFAULT '',
                 recovery_ref TEXT,
                 PRIMARY KEY (operation_id, item_index)
             );
             CREATE TABLE recovery_entries (
                 recovery_ref TEXT PRIMARY KEY,
                 operation_id TEXT NOT NULL,
                 scope_id TEXT NOT NULL,
                 original_locator TEXT NOT NULL,
                 quarantine_locator TEXT NOT NULL,
                 identity TEXT NOT NULL,
                 created_at_unix_ms INTEGER NOT NULL,
                 state TEXT NOT NULL
             );
             CREATE INDEX recovery_by_scope ON recovery_entries (scope_id, state);
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Inserts a scope row with a caller-chosen id. Used by the execution
    /// layer's fixtures so plans and operations can reference it directly.
    #[cfg(test)]
    pub(crate) fn insert_scope_row(
        &mut self,
        scope_id: &str,
        root: &diskgraph_core::Locator,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO scopes (scope_id, root_kind, root_raw_b64, root_display, volume_id, created_at_unix_ms, revoked)
             VALUES (?1, 'native_path', ?2, ?3, NULL, ?4, 0)",
            params![
                scope_id,
                root.raw_b64,
                root.display,
                Self::now_ms() as i64,
            ],
        )?;
        Ok(())
    }

    /// Runs one closure against the store's connection, so the execution layer
    /// keeps plans, approvals, operations, and recovery in the same control
    /// database as scopes and jobs.
    pub(crate) fn with_connection<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        work(&self.connection)
    }

    /// Mints the server identity on first use and returns the stored one after.
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
    pub fn revoke_scope(&mut self, scope_id: &ScopeId) -> Result<()> {
        self.scope(scope_id)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE scopes SET revoked = 1 WHERE scope_id = ?1",
            [scope_id.as_str()],
        )?;
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

    /// Current policy version (0 until the first publish).
    pub fn policy_version(&self) -> Result<u64> {
        let row: Option<(i64, i64)> = self
            .connection
            .query_row(
                "SELECT version, revoked FROM policy WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match row {
            Some((version, 0)) => Ok(version.max(0) as u64),
            _ => Ok(0),
        }
    }

    /// Publishes a new policy version; older grants stop applying.
    pub fn publish_policy_version(&mut self, version: u64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO policy (id, version, revoked) VALUES (1, ?1, 0)
             ON CONFLICT(id) DO UPDATE SET version = ?1, revoked = 0",
            [version as i64],
        )?;
        Ok(())
    }

    /// Revokes the whole policy; nothing is authorized until republished.
    pub fn revoke_policy(&mut self) -> Result<()> {
        self.connection
            .execute("UPDATE policy SET revoked = 1 WHERE id = 1", [])?;
        Ok(())
    }

    /// Upserts one grant bound to the current policy version.
    pub fn upsert_grant(&mut self, grant: &Grant) -> Result<()> {
        // 重复本地 bootstrap 只读已有键，避免每个查询进程重写权限和争夺提交锁。
        let existing: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM grants WHERE principal_id=?1 AND permission=?2 AND scope_id=?3 AND policy_version=?4)",
            params![grant.principal.as_str(), grant.permission.wire_name(), grant.scope.as_str(), grant.policy_version as i64],
            |row| row.get(0),
        )?;
        if existing {
            return Ok(());
        }
        self.connection.execute(
            "INSERT INTO grants (principal_id, permission, scope_id, policy_version)
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
            params![
                grant.principal.as_str(),
                grant.permission.wire_name(),
                grant.scope.as_str(),
                grant.policy_version as i64,
            ],
        )?;
        Ok(())
    }

    /// Withdraws a grant across every policy version it was recorded under.
    ///
    /// The table's key includes the version, so a grant can exist more than
    /// once; removing only the current one would leave the right standing
    /// under an epoch that has not been reached yet.
    pub fn revoke_grant(
        &mut self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<()> {
        self.connection.execute(
            "DELETE FROM grants
             WHERE principal_id = ?1 AND permission = ?2 AND scope_id = ?3",
            params![principal.as_str(), permission.wire_name(), scope.as_str(),],
        )?;
        Ok(())
    }

    /// The raw policy row: None when never published, Some((version, revoked))
    /// after. Distinct from `policy_version`, which flattens revocation into 0
    /// and therefore cannot distinguish "never published" from "revoked".
    pub fn policy_state(&self) -> Result<Option<(u64, bool)>> {
        let row: Option<(i64, i64)> = self
            .connection
            .query_row(
                "SELECT version, revoked FROM policy WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(row.map(|(version, revoked)| (version.max(0) as u64, revoked != 0)))
    }

    /// 查询一个权限的实时授权；无持久策略时返回 None，供可信内部兼容调用使用。
    pub fn live_permission(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<Option<bool>> {
        let revoked = self.scope(scope)?.revoked;
        if revoked {
            return Ok(Some(false));
        }
        if self.policy_state()?.is_none() {
            return Ok(None);
        }
        Ok(Some(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM policy p JOIN grants g ON g.policy_version = p.version WHERE p.id = 1 AND p.revoked = 0 AND g.principal_id = ?1 AND g.permission = ?2 AND g.scope_id = ?3)",params![principal.as_str(),permission.wire_name(),scope.as_str()],|row| row.get(0))?))
    }

    /// Builds the live authorizer from stored policy and grants.
    pub fn authorizer(&self) -> Result<PolicyAuthorizer> {
        let version = self.policy_version()?;
        let mut authorizer = PolicyAuthorizer::new(version);
        if let Some((_, true)) = self.policy_state()? {
            authorizer.revoke();
        }
        if version == 0 {
            return Ok(authorizer);
        }
        let mut statement = self
            .connection
            .prepare("SELECT principal_id, permission, scope_id, policy_version FROM grants")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (principal, permission, scope, policy_version) = row?;
            let Ok(principal) = PrincipalId::new(principal) else {
                continue;
            };
            let Ok(scope) = ScopeId::new(scope) else {
                continue;
            };
            let Some(permission) = parse_permission(&permission) else {
                continue;
            };
            authorizer.grant_at_version(principal, permission, scope, policy_version.max(0) as u64);
        }
        Ok(authorizer)
    }

    /// Creates a durable job for a live scope. An active job for the same
    /// scope is returned instead of duplicated (RT-01 job merging, AI-03).
    /// The creating principal is recorded so per-principal quotas can count it.
    pub fn create_job(
        &mut self,
        scope_id: &ScopeId,
        kind: JobKind,
        principal: &PrincipalId,
    ) -> Result<JobRecord> {
        self.create_job_internal(scope_id, kind, principal, u64::MAX, false)?
            .ok_or_else(|| StoreError::Conflict("job quota exceeded".into()))
    }

    /// 在单个写事务内按真实主体合并并检查配额，防止跨进程重复入队。
    pub fn create_job_with_quota(
        &mut self,
        scope_id: &ScopeId,
        kind: JobKind,
        principal: &PrincipalId,
        maximum: u64,
    ) -> Result<Option<JobRecord>> {
        self.create_job_internal(scope_id, kind, principal, maximum, true)
    }

    fn create_job_internal(
        &mut self,
        scope_id: &ScopeId,
        kind: JobKind,
        principal: &PrincipalId,
        maximum: u64,
        require_live_grant: bool,
    ) -> Result<Option<JobRecord>> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let revoked: bool = tx
            .query_row(
                "SELECT revoked FROM scopes WHERE scope_id = ?1",
                [scope_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::ScopeNotFound(scope_id.as_str().to_owned()))?;
        if revoked {
            return Err(StoreError::Conflict(format!("scope {scope_id} is revoked")));
        }
        if require_live_grant {
            let authorized: bool = tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM policy WHERE id = 1) OR EXISTS(SELECT 1 FROM policy p JOIN grants g ON g.policy_version = p.version WHERE p.id = 1 AND p.revoked = 0 AND g.principal_id = ?1 AND g.scope_id = ?2 AND g.permission = 'index:write')",params![principal.as_str(), scope_id.as_str()],|row| row.get(0))?;
            if !authorized {
                return Err(StoreError::Conflict(
                    "live index authorization was withdrawn".into(),
                ));
            }
        }
        let existing: Option<String>=tx.query_row("SELECT job_id FROM jobs WHERE scope_id = ?1 AND principal = ?2 AND state IN ('queued','running') ORDER BY created_at_unix_ms DESC, job_id DESC LIMIT 1",params![scope_id.as_str(),principal.as_str()],|row| row.get(0)).optional()?;
        if let Some(job_id) = existing {
            tx.commit()?;
            return self.job(&job_id).map(Some);
        }
        let active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM jobs WHERE principal = ?1 AND state IN ('queued','running')",
            [principal.as_str()],
            |row| row.get(0),
        )?;
        if active.max(0) as u64 >= maximum {
            return Ok(None);
        }
        let job_id = format!("job-{}", Uuid::new_v4());
        let now = Self::now_ms();
        tx.execute("INSERT INTO jobs (job_id, scope_id, kind, state, created_at_unix_ms, heartbeat_unix_ms, owner, principal) VALUES (?1,?2,?3,'queued',?4,?4,'',?5)",params![job_id,scope_id.as_str(),kind.as_str(),now as i64,principal.as_str()])?;
        tx.commit()?;
        self.job(&job_id).map(Some)
    }

    /// Whether this principal holds scope administration on the admin scope
    /// under any grant epoch. Management operations (publish/revoke) validate
    /// against this instead of the live authorizer, so a revoked policy can
    /// still be republished by its administrator (SC-04 break-glass).
    pub fn holds_admin(&self, principal: &PrincipalId) -> Result<bool> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM grants
             WHERE principal_id = ?1 AND permission = 'scope:admin'
               AND scope_id = 'diskgraph-admin'",
            [principal.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Every stored grant, for epoch renewal at bootstrap.
    pub fn all_grants(&self) -> Result<Vec<Grant>> {
        let mut statement = self
            .connection
            .prepare("SELECT principal_id, permission, scope_id, policy_version FROM grants")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut grants = Vec::new();
        for row in rows {
            let (principal, permission, scope, version) = row?;
            let Some(permission) = parse_permission(&permission) else {
                continue;
            };
            let Ok(principal) = PrincipalId::new(principal) else {
                continue;
            };
            let Ok(scope) = ScopeId::new(scope) else {
                continue;
            };
            grants.push(Grant {
                principal,
                permission,
                scope,
                policy_version: version.max(0) as u64,
            });
        }
        Ok(grants)
    }

    /// Every queued job, oldest first, across scopes. The job runner drains
    /// this list; nothing here is tied to a connection.
    pub fn list_queued_jobs(&self) -> Result<Vec<JobRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT jobs.job_id FROM jobs JOIN scopes ON scopes.scope_id = jobs.scope_id
             WHERE scopes.revoked = 0 AND jobs.cancel_requested = 0
               AND (jobs.state = 'queued' OR (jobs.state = 'running' AND jobs.lease_expires_unix_ms <= ?1))
             ORDER BY jobs.created_at_unix_ms ASC, jobs.job_id ASC",
        )?;
        let rows = statement.query_map([Self::now_ms() as i64], |row| row.get::<_, String>(0))?;
        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(self.job(&row?)?);
        }
        Ok(jobs)
    }

    /// Ends expired jobs that revocation or cancellation made unclaimable.
    pub fn reap_unclaimable_jobs(&mut self) -> Result<u64> {
        let changed = self.connection.execute(
            "UPDATE jobs SET state = 'cancelled', heartbeat_unix_ms = ?1
             WHERE state = 'running' AND lease_expires_unix_ms <= ?1
               AND (cancel_requested = 1 OR EXISTS
                   (SELECT 1 FROM scopes WHERE scopes.scope_id = jobs.scope_id AND scopes.revoked = 1))",
            [Self::now_ms() as i64],
        )?;
        Ok(changed as u64)
    }

    /// 仅终结指定 ID 的过期且因取消/范围撤销不可认领任务。
    /// 返回是否条件更新成功；存活租约、已完成和其他任务均保持不变。
    pub fn reap_unclaimable_job(&mut self, job_id: &str) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE jobs SET state = 'cancelled', heartbeat_unix_ms = ?2
             WHERE job_id = ?1 AND state = 'running' AND lease_expires_unix_ms <= ?2
               AND (cancel_requested = 1 OR EXISTS
                   (SELECT 1 FROM scopes WHERE scopes.scope_id = jobs.scope_id AND scopes.revoked = 1))",
            params![job_id, Self::now_ms() as i64],
        )?;
        Ok(changed == 1)
    }

    /// Active (queued or running) jobs attributed to one principal. Merged
    /// jobs count once, because the merge returns the existing record.
    pub fn active_job_count_for_principal(&self, principal: &PrincipalId) -> Result<u64> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM jobs
             WHERE principal = ?1 AND state IN ('queued', 'running')",
            [principal.as_str()],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    /// Loads one job record.
    pub fn job(&self, job_id: &str) -> Result<JobRecord> {
        self.connection
            .query_row(
                "SELECT scope_id, kind, state, created_at_unix_ms, heartbeat_unix_ms, owner, principal, fencing_token, lease_expires_unix_ms
                 FROM jobs WHERE job_id = ?1",
                [job_id],
                |row| {
                    let principal = row.get::<_, String>(6).unwrap_or_default();
                    let principal = PrincipalId::new(if principal.is_empty() { "legacy-unbound".to_owned() } else { principal }).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?;
                    Ok(JobRecord {
                        job_id: job_id.to_owned(),
                        scope_id: ScopeId::new(row.get::<_, String>(0)?).map_err(|error| {
                            rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                        })?,
                        kind: JobKind::from_str(&row.get::<_, String>(1)?).ok_or(
                            rusqlite::Error::InvalidColumnType(
                                1,
                                "kind".into(),
                                rusqlite::types::Type::Text,
                            ),
                        )?,
                        state: JobState::from_str(&row.get::<_, String>(2)?).ok_or(
                            rusqlite::Error::InvalidColumnType(
                                2,
                                "state".into(),
                                rusqlite::types::Type::Text,
                            ),
                        )?,
                        created_at_unix_ms: row.get::<_, i64>(3)?.try_into().unwrap_or(0),
                        heartbeat_unix_ms: row.get::<_, i64>(4)?.try_into().unwrap_or(0),
                        owner: row.get(5)?,
                        fencing_token: row.get::<_, i64>(7)?.max(0) as u64,
                        lease_expires_unix_ms: row.get::<_, i64>(8)?.max(0) as u64,
                        principal,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::JobNotFound(job_id.to_owned()))
    }

    /// The one active (queued or running) job for a scope, newest first.
    pub fn active_job_for_scope(&self, scope_id: &ScopeId) -> Result<Option<JobRecord>> {
        let job_id: Option<String> = self
            .connection
            .query_row(
                "SELECT job_id FROM jobs
                 WHERE scope_id = ?1 AND state IN ('queued', 'running')
                 ORDER BY created_at_unix_ms DESC, job_id DESC LIMIT 1",
                [scope_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match job_id {
            Some(job_id) => Ok(Some(self.job(&job_id)?)),
            None => Ok(None),
        }
    }

    /// 操作记录未绑定 revision 的旧模型保守保留整个 scope；事务阻止并发新引用。
    pub fn with_retention_guard<T>(
        &mut self,
        scope_id: &ScopeId,
        work: impl FnOnce(bool) -> Result<T>,
    ) -> Result<T> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let referenced: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM plans WHERE scope_id = ?1 UNION ALL SELECT 1 FROM operations WHERE scope_id = ?1 UNION ALL SELECT 1 FROM recovery_entries WHERE scope_id = ?1)", [scope_id.as_str()], |row| row.get(0))?;
        let result = work(!referenced)?;
        tx.commit()?;
        Ok(result)
    }

    /// Transitions a queued job to running under `owner` (fencing token).
    /// A running job owned by someone else refuses the claim.
    pub fn claim_job(&mut self, job_id: &str, owner: &str) -> Result<JobRecord> {
        self.claim_job_internal(job_id, owner, true)
    }

    /// runner 严格认领；同名 owner 也不能再次取得尚未过期的 running job。
    /// 参数为任务 ID 与本次执行 owner，返回唯一认领代次。
    pub fn claim_job_once(&mut self, job_id: &str, owner: &str) -> Result<JobRecord> {
        self.claim_job_internal(job_id, owner, false)
    }

    fn claim_job_internal(
        &mut self,
        job_id: &str,
        owner: &str,
        trusted_idempotent: bool,
    ) -> Result<JobRecord> {
        let now = Self::now_ms();
        let changed = self.connection.execute(
            "UPDATE jobs SET state = 'running', owner = ?2, heartbeat_unix_ms = ?3, lease_expires_unix_ms = ?4, fencing_token = fencing_token + 1 WHERE job_id = ?1 AND cancel_requested = 0 AND (state = 'queued' OR (state = 'running' AND lease_expires_unix_ms <= ?3)) AND EXISTS (SELECT 1 FROM scopes WHERE scopes.scope_id = jobs.scope_id AND revoked = 0)",
            params![job_id, owner, now as i64, now.saturating_add(30000) as i64],
        )?;
        let job = self.job(job_id)?;
        if changed == 1 {
            return Ok(job);
        }
        if trusted_idempotent
            && job.state == JobState::Running
            && job.owner == owner
            && job.lease_expires_unix_ms > now
        {
            return Ok(job);
        }
        if job.state == JobState::Running {
            return Err(StoreError::StaleOwner);
        }
        Err(StoreError::Conflict(format!(
            "job {job_id} is not claimable"
        )))
    }

    /// 刷新当前 owner 的租约；已过期的 owner 不得续租。
    pub fn heartbeat(&mut self, job_id: &str, owner: &str) -> Result<()> {
        let job = self.job(job_id)?;
        self.heartbeat_fenced(job_id, owner, job.fencing_token)
    }

    /// 携带 fencing token 续租，防止同名 owner 的旧请求复活。
    pub fn heartbeat_fenced(&mut self, job_id: &str, owner: &str, fence: u64) -> Result<()> {
        let now = Self::now_ms();
        let changed = self.connection.execute("UPDATE jobs SET heartbeat_unix_ms = ?4, lease_expires_unix_ms = ?5 WHERE job_id = ?1 AND owner = ?2 AND fencing_token = ?3 AND state = 'running' AND cancel_requested = 0 AND lease_expires_unix_ms > ?4", params![job_id, owner, fence as i64, now as i64, now.saturating_add(30000) as i64])?;
        if changed != 1 {
            return Err(StoreError::StaleOwner);
        }
        Ok(())
    }

    /// 在控制库写事务保护下验证租约并执行 staging/发布，跨进程认领不能插入中间。
    pub fn with_job_fence<T>(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
        work: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let valid: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM jobs j JOIN scopes s ON s.scope_id = j.scope_id WHERE j.job_id = ?1 AND j.owner = ?2 AND j.fencing_token = ?3 AND j.state = 'running' AND j.cancel_requested = 0 AND j.lease_expires_unix_ms > ?4 AND s.revoked = 0 AND (NOT EXISTS (SELECT 1 FROM policy WHERE id = 1) OR EXISTS (SELECT 1 FROM policy p JOIN grants g ON g.policy_version = p.version WHERE p.id = 1 AND p.revoked = 0 AND g.principal_id = j.principal AND g.scope_id = j.scope_id AND g.permission = 'index:write')))", params![job_id, owner, fence as i64, Self::now_ms() as i64], |row| row.get(0))?;
        if !valid {
            return Err(StoreError::StaleOwner);
        }
        let result = work()?;
        tx.commit()?;
        Ok(result)
    }

    /// Persists cancellation across Engine instances. Terminal jobs are unchanged.
    pub fn request_cancel(&mut self, job_id: &str) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE jobs SET cancel_requested = 1,
                 state = CASE WHEN state = 'queued' THEN 'cancelled' ELSE state END,
                 heartbeat_unix_ms = ?2
             WHERE job_id = ?1 AND state IN ('queued', 'running')",
            params![job_id, Self::now_ms() as i64],
        )?;
        if changed == 0 {
            self.job(job_id)?;
        }
        Ok(changed > 0)
    }

    /// Reads cancellation for the exact claim; stale generations are stopped.
    pub fn cancellation_requested(&self, job_id: &str, fence: u64) -> Result<bool> {
        let cancelled: Option<bool> = self
            .connection
            .query_row(
                "SELECT cancel_requested != 0 OR state = 'cancelled' FROM jobs
             WHERE job_id = ?1 AND fencing_token = ?2",
                params![job_id, fence as i64],
                |row| row.get(0),
            )
            .optional()?;
        Ok(cancelled.unwrap_or(true))
    }

    /// Cancels a job that is still queued. Returns whether it was cancelled;
    /// running jobs are the runner's responsibility (RT-02).
    pub fn cancel_queued(&mut self, job_id: &str) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE jobs SET state = 'cancelled', heartbeat_unix_ms = ?2
             WHERE job_id = ?1 AND state = 'queued'",
            params![job_id, Self::now_ms() as i64],
        )?;
        Ok(changed > 0)
    }

    /// Moves a running job to a terminal state; only the current owner may.
    pub fn finish_job(&mut self, job_id: &str, owner: &str, state: JobState) -> Result<JobRecord> {
        let job = self.job(job_id)?;
        self.finish_job_fenced(job_id, owner, job.fencing_token, state)
    }

    /// 仅当前租约代次可结束任务；同名 owner 的旧代次也被拒绝。
    pub fn finish_job_fenced(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
        state: JobState,
    ) -> Result<JobRecord> {
        if !matches!(
            state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return Err(StoreError::Conflict(
                "finish_job requires a terminal state".into(),
            ));
        }
        let changed = self.connection.execute(
            "UPDATE jobs SET state = ?2, heartbeat_unix_ms = ?3 WHERE job_id = ?1 AND owner = ?4 AND fencing_token = ?5 AND state = 'running' AND lease_expires_unix_ms > ?3",
            params![job_id, state.as_str(), Self::now_ms() as i64, owner, fence as i64],
        )?;
        if changed != 1 {
            return Err(StoreError::StaleOwner);
        }
        self.job(job_id)
    }
}

fn locator_kind_tag(kind: LocatorKind) -> &'static str {
    match kind {
        LocatorKind::NativePath => "native_path",
        LocatorKind::DocumentUri => "document_uri",
    }
}

fn parse_locator_kind(value: &str) -> LocatorKind {
    match value {
        "document_uri" => LocatorKind::DocumentUri,
        _ => LocatorKind::NativePath,
    }
}

fn parse_permission(value: &str) -> Option<Permission> {
    Some(match value {
        "metadata:read" => Permission::MetadataRead,
        "content:read" => Permission::ContentRead,
        "index:write" => Permission::IndexWrite,
        "scope:admin" => Permission::ScopeAdmin,
        "operations:view" => Permission::OperationView,
        "files:move" => Permission::FileAction(FileActionKind::Move),
        "files:copy" => Permission::FileAction(FileActionKind::Copy),
        "files:trash" => Permission::FileAction(FileActionKind::Trash),
        "files:restore" => Permission::FileAction(FileActionKind::Restore),
        "files:purge" => Permission::FileAction(FileActionKind::Purge),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_repeated_grant_does_not_require_a_write_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control.sqlite");
        let mut store = super::ControlStore::open(&path).unwrap();
        let grant = diskgraph_core::Grant {
            principal: diskgraph_core::PrincipalId::new("local").unwrap(),
            permission: diskgraph_core::Permission::MetadataRead,
            scope: diskgraph_core::ScopeId::new("fixture").unwrap(),
            policy_version: 1,
        };
        store.upsert_grant(&grant).unwrap();
        let observer = rusqlite::Connection::open(&path).unwrap();
        observer
            .execute_batch("BEGIN; SELECT * FROM grants;")
            .unwrap();
        store
            .connection
            .busy_timeout(std::time::Duration::from_millis(20))
            .unwrap();
        let changed = store.connection.total_changes();
        // 活跃共享读事务不妨碍幂等 bootstrap；旧 REPLACE 会等待独占提交并失败。
        store.upsert_grant(&grant).unwrap();
        assert_eq!(store.connection.total_changes(), changed);
        observer.execute_batch("ROLLBACK;").unwrap();
    }
    use super::*;
    use diskgraph_core::{Authorizer as _, LocatorKind, Permission};

    fn native_root(name: &str) -> Locator {
        Locator {
            kind: LocatorKind::NativePath,
            raw_b64: name.to_owned(),
            display: format!("/display/{name}"),
        }
    }

    fn scope(name: &str) -> ScopeId {
        ScopeId::new(name).unwrap()
    }

    fn principal(name: &str) -> PrincipalId {
        PrincipalId::new(name).unwrap()
    }

    #[test]
    fn server_identity_is_minted_once_and_persisted() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("control.sqlite");
        let first = {
            let mut store = ControlStore::open(&db).unwrap();
            store.ensure_server().unwrap()
        };
        let second = {
            let mut store = ControlStore::open(&db).unwrap();
            store.ensure_server().unwrap()
        };
        assert_eq!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn control_database_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("control.sqlite");
        ControlStore::open(&db).unwrap();
        let mode = std::fs::metadata(&db).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "control records must not be world-readable");
    }

    #[test]
    fn scope_registration_is_idempotent_until_revoked() {
        let mut store = ControlStore::open_in_memory().unwrap();
        let first = store
            .register_scope(&native_root("one"), Some("vol-1"))
            .unwrap();
        let second = store
            .register_scope(&native_root("one"), Some("vol-1"))
            .unwrap();
        assert_eq!(first, second);

        store.revoke_scope(&first).unwrap();
        assert!(store.scope(&first).unwrap().revoked);
        assert!(matches!(
            store.register_scope(&native_root("one"), None),
            Err(StoreError::Conflict(_))
        ));

        let other = store.register_scope(&native_root("two"), None).unwrap();
        assert_ne!(first, other);
        assert_eq!(store.list_scopes().unwrap().len(), 2);
    }

    #[test]
    fn revoked_scopes_reject_new_jobs_but_records_survive() {
        let mut store = ControlStore::open_in_memory().unwrap();
        let scope_id = store.register_scope(&native_root("project"), None).unwrap();
        let job = store
            .create_job(&scope_id, JobKind::Index, &principal("agent"))
            .unwrap();
        store.revoke_scope(&scope_id).unwrap();
        assert!(matches!(
            store.create_job(&scope_id, JobKind::Sync, &principal("agent")),
            Err(StoreError::Conflict(_))
        ));
        // Revocation never erases history: the old job stays queryable.
        assert_eq!(store.job(&job.job_id).unwrap().scope_id, scope_id);
        // Revoking again is idempotent.
        store.revoke_scope(&scope_id).unwrap();
    }

    #[test]
    fn cancellation_is_durable_and_invalidates_the_running_publish_fence() {
        let mut store = ControlStore::open_in_memory().unwrap();
        let scope_id = store.register_scope(&native_root("cancel"), None).unwrap();
        let job = store
            .create_job(&scope_id, JobKind::Index, &principal("agent"))
            .unwrap();
        let claimed = store.claim_job_once(&job.job_id, "worker").unwrap();
        assert!(store.request_cancel(&job.job_id).unwrap());
        assert!(
            store
                .cancellation_requested(&job.job_id, claimed.fencing_token)
                .unwrap()
        );
        assert!(matches!(
            store.with_job_fence(&job.job_id, "worker", claimed.fencing_token, || Ok(())),
            Err(StoreError::StaleOwner)
        ));
        let terminal = store
            .finish_job_fenced(
                &job.job_id,
                "worker",
                claimed.fencing_token,
                JobState::Cancelled,
            )
            .unwrap();
        assert_eq!(terminal.state, JobState::Cancelled);
    }

    #[test]
    fn v4_jobs_migrate_to_persistent_cancellation_with_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control.sqlite");
        let mut first = ControlStore::open(&path).unwrap();
        let scope_id = first
            .register_scope(&native_root("migration"), None)
            .unwrap();
        let job = first
            .create_job(&scope_id, JobKind::Index, &principal("agent"))
            .unwrap();
        drop(first);
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "ALTER TABLE jobs DROP COLUMN cancel_requested; PRAGMA user_version = 4;",
            )
            .unwrap();
        drop(connection);
        let mut migrated = ControlStore::open(&path).unwrap();
        assert_eq!(migrated.job(&job.job_id).unwrap().state, JobState::Queued);
        assert!(migrated.request_cancel(&job.job_id).unwrap());
        assert_eq!(
            migrated.job(&job.job_id).unwrap().state,
            JobState::Cancelled
        );
        assert!(
            directory
                .path()
                .join("migration_backups/control.sqlite.pre-v5.bak")
                .is_file()
        );
    }

    #[test]
    fn revoked_expired_job_does_not_precede_eligible_work_forever() {
        let mut store = ControlStore::open_in_memory().unwrap();
        let revoked = store.register_scope(&native_root("revoked"), None).unwrap();
        let eligible = store
            .register_scope(&native_root("eligible"), None)
            .unwrap();
        let first = store
            .create_job(&revoked, JobKind::Index, &principal("agent"))
            .unwrap();
        let claimed = store.claim_job_once(&first.job_id, "dead-worker").unwrap();
        store
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE jobs SET lease_expires_unix_ms = 0 WHERE job_id = ?1",
                    [&first.job_id],
                )?;
                Ok(())
            })
            .unwrap();
        store.revoke_scope(&revoked).unwrap();
        let second = store
            .create_job(&eligible, JobKind::Index, &principal("agent"))
            .unwrap();
        let candidates = store.list_queued_jobs().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].job_id, second.job_id);
        assert_eq!(store.job(&first.job_id).unwrap().state, JobState::Cancelled);
        assert_eq!(claimed.state, JobState::Running);
    }

    #[test]
    fn policy_grants_round_trip_through_storage_and_rebuild_the_authorizer() {
        let mut store = ControlStore::open_in_memory().unwrap();
        store.publish_policy_version(3).unwrap();
        store
            .upsert_grant(&Grant {
                principal: principal("agent"),
                permission: Permission::MetadataRead,
                scope: scope("project"),
                policy_version: 3,
            })
            .unwrap();
        // Older-version grant recorded underneath the current version.
        store
            .upsert_grant(&Grant {
                principal: principal("agent"),
                permission: Permission::ContentRead,
                scope: scope("project"),
                policy_version: 2,
            })
            .unwrap();

        let authorizer = store.authorizer().unwrap();
        assert!(matches!(
            authorizer.decide(
                &principal("agent"),
                &Permission::MetadataRead,
                &scope("project")
            ),
            diskgraph_core::Decision::Allowed
        ));
        assert!(matches!(
            authorizer.decide(
                &principal("agent"),
                &Permission::ContentRead,
                &scope("project")
            ),
            diskgraph_core::Decision::Denied(diskgraph_core::DenyReason::PolicyVersionMismatch)
        ));

        store.revoke_policy().unwrap();
        let authorizer = store.authorizer().unwrap();
        assert!(matches!(
            authorizer.decide(
                &principal("agent"),
                &Permission::MetadataRead,
                &scope("project")
            ),
            diskgraph_core::Decision::Denied(_)
        ));
    }

    #[test]
    fn jobs_merge_per_scope_and_enforce_owner_fencing() {
        let mut store = ControlStore::open_in_memory().unwrap();
        let scope_id = store.register_scope(&native_root("project"), None).unwrap();
        let first = store
            .create_job(&scope_id, JobKind::Index, &principal("agent"))
            .unwrap();
        let merged = store
            .create_job(&scope_id, JobKind::Sync, &principal("agent"))
            .unwrap();
        assert_eq!(
            first.job_id, merged.job_id,
            "active jobs must merge per scope"
        );

        let job_id = first.job_id.clone();
        let claimed = store.claim_job(&job_id, "worker-a").unwrap();
        assert_eq!(claimed.state, JobState::Running);
        assert_eq!(claimed.owner, "worker-a");

        // Fencing: another owner can neither claim, heartbeat, nor finish.
        assert!(matches!(
            store.claim_job(&job_id, "worker-b").unwrap_err(),
            StoreError::StaleOwner
        ));
        assert!(matches!(
            store.heartbeat(&job_id, "worker-b").unwrap_err(),
            StoreError::StaleOwner
        ));
        assert!(matches!(
            store
                .finish_job(&job_id, "worker-b", JobState::Completed)
                .unwrap_err(),
            StoreError::StaleOwner
        ));

        store.heartbeat(&job_id, "worker-a").unwrap();
        let finished = store
            .finish_job(&job_id, "worker-a", JobState::Completed)
            .unwrap();
        assert_eq!(finished.state, JobState::Completed);
        assert!(store.active_job_for_scope(&scope_id).unwrap().is_none());
        assert!(matches!(
            store.claim_job(&job_id, "worker-a"),
            Err(StoreError::Conflict(_))
        ));
    }

    #[test]
    fn unknown_scope_and_job_ids_are_reported_not_invented() {
        let store = ControlStore::open_in_memory().unwrap();
        assert!(matches!(
            store.scope(&scope("missing")),
            Err(StoreError::ScopeNotFound(_))
        ));
        assert!(matches!(
            store.job("job-none"),
            Err(StoreError::JobNotFound(_))
        ));
    }
}
