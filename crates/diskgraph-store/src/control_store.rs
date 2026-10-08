//! 控制库连接、一致性迁移备份与内部事务访问。

use crate::control_store_incarnation::ControlStoreIncarnation;
use crate::{Result, StoreError};
use rusqlite::Connection;
#[cfg(test)]
use rusqlite::params;
use std::path::Path;
use std::sync::Arc;

/// 保存身份、授权、任务、操作与恢复记录的持久控制库。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// Control-plane database: identity, scopes, policy, and jobs.
pub struct ControlStore {
    // 字段按声明顺序释放：先使 Weak 代次失效，再关闭 SQLite 原生文件。
    pub(crate) withdrawal_incarnation: Arc<ControlStoreIncarnation>,
    pub(crate) connection: Connection,
}

impl ControlStore {
    /// Opens (creating if needed) a control database. On Unix the file is
    /// restricted to owner-only permissions.
    /// 打开或初始化连接，遵循各自只读、期限、备份和版本门禁。
    /// 参数：path：数据库原生路径。
    /// 返回：配置和版本校验成功的存储连接，失败不返回可用存储。
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if (1..9).contains(&version) {
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
            crate::migration_backup::create_migration_backup(
                &connection,
                &backup_dir.join(format!(
                    "{name}.pre-v{}.bak",
                    if version < 4 {
                        4
                    } else if version < 5 {
                        5
                    } else if version < 6 {
                        6
                    } else if version < 7 {
                        7
                    } else if version < 8 {
                        8
                    } else {
                        9
                    }
                )),
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

    /// 打开或初始化连接，遵循各自只读、期限、备份和版本门禁。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：配置和版本校验成功的存储连接，失败不返回可用存储。
    pub fn open_in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self> {
        // `version` is the schema version we migrate FROM; it advances as each
        // migration runs, so a fresh database walks exactly the same path an
        // older file would and never skips a step.
        let mut version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if !(0..=9).contains(&version) {
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
        if version < 6 {
            Self::migrate_authorization_generation(&connection)?;
            version = 6;
        }
        if version < 7 {
            Self::migrate_job_request_authorities(&connection)?;
            version = 7;
        }
        if version < 8 {
            Self::migrate_git_job_inputs(&connection)?;
            version = 8;
        }
        if version < 9 {
            Self::migrate_process_job_inputs(&connection)?;
            version = 9;
        }
        debug_assert_eq!(version, 9, "every control migration must have run");
        Self::validate_job_authority_schema(&connection)?;
        Self::validate_git_job_input_schema(&connection)?;
        Self::validate_process_job_input_schema(&connection)?;
        let withdrawal_incarnation = Arc::new(ControlStoreIncarnation::new(&connection));
        Ok(Self {
            withdrawal_incarnation,
            connection,
        })
    }

    /// 生成持久时间或非法状态诊断。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：Unix 毫秒时间。
    pub(crate) fn now_ms() -> u64 {
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
    /// 持久管理稳定服务器身份及范围注册、读取和撤销。
    /// 参数：scope_id：实际所属范围 ID；root：无损根定位或根过滤条件。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
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
    /// 在当前控制连接内执行内部回调。
    /// 参数：work：接收同一控制连接的内部回调；需要事务时由回调建立。
    /// 返回：当前控制连接回调结果，不新增连接或跨库原子承诺。
    pub(crate) fn with_connection<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        work(&self.connection)
    }
}
