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
        if !(0..=3).contains(&version) {
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
        debug_assert_eq!(version, 3, "every control migration must have run");
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
        self.connection.execute(
            "UPDATE scopes SET revoked = 1 WHERE scope_id = ?1",
            [scope_id.as_str()],
        )?;
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
        self.connection.execute(
            "INSERT OR REPLACE INTO grants (principal_id, permission, scope_id, policy_version)
             VALUES (?1, ?2, ?3, ?4)",
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
        let scope = self.scope(scope_id)?;
        if scope.revoked {
            return Err(StoreError::Conflict(format!(
                "scope {} is revoked",
                scope_id.as_str()
            )));
        }
        if let Some(active) = self.active_job_for_scope(scope_id)? {
            return Ok(active);
        }
        let job_id = format!("job-{}", Uuid::new_v4());
        let now = Self::now_ms();
        self.connection.execute(
            "INSERT INTO jobs (job_id, scope_id, kind, state, created_at_unix_ms, heartbeat_unix_ms, owner, principal)
             VALUES (?1, ?2, ?3, 'queued', ?4, ?4, '', ?5)",
            params![job_id, scope_id.as_str(), kind.as_str(), now as i64, principal.as_str()],
        )?;
        self.job(&job_id)
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
            "SELECT job_id FROM jobs WHERE state = 'queued'
             ORDER BY created_at_unix_ms ASC, job_id ASC",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(self.job(&row?)?);
        }
        Ok(jobs)
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
                "SELECT scope_id, kind, state, created_at_unix_ms, heartbeat_unix_ms, owner, principal
                 FROM jobs WHERE job_id = ?1",
                [job_id],
                |row| {
                    let principal = row.get::<_, String>(6).unwrap_or_default();
                    let principal = PrincipalId::new(principal).map_err(|error| {
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

    /// Transitions a queued job to running under `owner` (fencing token).
    /// A running job owned by someone else refuses the claim.
    pub fn claim_job(&mut self, job_id: &str, owner: &str) -> Result<JobRecord> {
        let job = self.job(job_id)?;
        match job.state {
            JobState::Queued => {}
            JobState::Running if job.owner == owner => return Ok(job),
            JobState::Running => return Err(StoreError::StaleOwner),
            JobState::Completed | JobState::Failed | JobState::Cancelled => {
                return Err(StoreError::Conflict(format!(
                    "job {job_id} already finished as {:?}",
                    job.state
                )));
            }
        }
        self.connection.execute(
            "UPDATE jobs SET state = 'running', owner = ?2, heartbeat_unix_ms = ?3
             WHERE job_id = ?1",
            params![job_id, owner, Self::now_ms() as i64],
        )?;
        self.job(job_id)
    }

    /// Refreshes the heartbeat; only the current owner may do so.
    pub fn heartbeat(&mut self, job_id: &str, owner: &str) -> Result<()> {
        let job = self.job(job_id)?;
        if job.state != JobState::Running || job.owner != owner {
            return Err(StoreError::StaleOwner);
        }
        self.connection.execute(
            "UPDATE jobs SET heartbeat_unix_ms = ?2 WHERE job_id = ?1",
            params![job_id, Self::now_ms() as i64],
        )?;
        Ok(())
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
        if job.owner != owner {
            return Err(StoreError::StaleOwner);
        }
        if !matches!(
            state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return Err(StoreError::Conflict(
                "finish_job requires a terminal state".into(),
            ));
        }
        self.connection.execute(
            "UPDATE jobs SET state = ?2, heartbeat_unix_ms = ?3 WHERE job_id = ?1",
            params![job_id, state.as_str(), Self::now_ms() as i64],
        )?;
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
