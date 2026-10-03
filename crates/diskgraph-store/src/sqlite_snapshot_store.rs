//! SQLite 连接、版本门禁、迁移入口与 WAL 生命周期。

use crate::directory_aggregates;
use crate::graph_migrations::{V1_SCHEMA, migrate_v1_to_v2, migrate_v2_to_v3, migrate_v3_to_v4};
use crate::{Result, StoreError};
use diskgraph_core::ResourceLocator;
use rusqlite::{Connection, params};
use serde_json::from_str;
use std::path::{Path, PathBuf};

/// 不可变观测图的 SQLite 存储，不操作扫描资源。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// A store owns only observed metadata. It never opens or removes scanned paths.
pub struct SqliteSnapshotStore {
    pub(crate) connection: Connection,
}

/// The newest schema this build understands; older binaries refuse newer files
/// through [`StoreError::UnsupportedSchema`] (design D6, spec ST-02).
pub const SUPPORTED_SCHEMA_VERSION: i64 = 10;

/// A WAL this size or larger is a leftover from a killed or out-of-memory
/// run: a healthy scan checkpoints as it goes, and a clean close removes
/// the file, so a session should never open onto one this big.
pub(crate) const WAL_HEAL_THRESHOLD_BYTES: i64 = 64 << 20;

/// The write-ahead log beside a database file, named the way SQLite names
/// it: the database path with a `-wal` suffix.
/// 定位或尝试回收数据库旁的 WAL，不以锁争用判作数据损坏。
/// 参数：database：数据库原生路径。
/// 返回：保留原生 OS 字节的数据库 -wal 路径。
pub(crate) fn wal_path(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_os_string();
    name.push("-wal");
    PathBuf::from(name)
}

impl SqliteSnapshotStore {
    /// 独立只读连接，不执行迁移、恢复或 checkpoint；SQLite 执行受期限和取消约束。
    /// 打开有期限与取消约束的只读连接，数据库版本由既有初始化流程确认。
    /// 参数：path：数据库原生路径；deadline_ms：查询执行期限，毫秒；cancel：共享取消标志。
    /// 返回：配置成功的只读存储连接；此入口不读取或校验 user_version，打开或配置失败返回错误。
    pub fn open_reader(
        path: &Path,
        deadline_ms: u64,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Self> {
        let deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_millis(deadline_ms))
            .ok_or_else(|| {
                StoreError::InvalidGraph("reader deadline is not representable".into())
            })?;
        Self::open_reader_until(path, deadline, cancel)
    }

    /// 独立只读连接继承请求的绝对期限，打开及配置不重新计时。
    /// 参数：path 为原生数据库路径，deadline 为首次准备前的期限，cancel 为取消标志。
    /// 返回：只读连接或打开/配置失败；同步调用只可协作检查，不保证硬实时。
    pub fn open_reader_until(
        path: &Path,
        deadline: std::time::Instant,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(std::time::Duration::from_secs(1)),
        )?;
        connection.pragma_update(None, "temp_store", "FILE")?;
        connection.pragma_update(None, "cache_size", -8192)?;
        connection.progress_handler(
            1000,
            Some(move || {
                std::time::Instant::now() >= deadline
                    || cancel
                        .as_ref()
                        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            }),
        )?;
        Ok(Self { connection })
    }

    /// 打开或初始化连接，遵循各自只读、期限、备份和版本门禁。
    /// 参数：path：数据库原生路径。
    /// 返回：配置和版本校验成功的存储连接，失败不返回可用存储。
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        // Independent CLI processes can open the same store together. The
        // schema/journal setup below may briefly need SQLite's writer lock.
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL + NORMAL: one fsync per checkpoint, not per commit — the
        // atomicity of staging/publish comes from the transaction, not from
        // per-commit fsyncs. WAL/NORMAL preserves consistency, but a power
        // failure can lose recently committed staging or published revisions.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        // 之前的 1 << 31 推断为 i32 负数，实际并未启用映射；明确维持关闭。
        // 大窗口 mmap 需要单独的平台/生命周期验收，当前读者使用 SQLite 页缓存。
        connection.pragma_update(None, "mmap_size", 0_i64)?;
        // Without a journal_size_limit the -wal file never shrinks below its
        // high-water mark: a checkpoint folds the frames back into the main
        // database but leaves the file at full size, so one scan's worth of
        // write-ahead log stays on disk for the life of the database. A
        // multi-million-node scan writes several gigabytes, so without this
        // the log ends up nearly as large as the snapshot it belongs to.
        connection.pragma_update(None, "journal_size_limit", WAL_HEAL_THRESHOLD_BYTES)?;
        Self::heal_oversized_wal(path, &connection, WAL_HEAL_THRESHOLD_BYTES)?;
        Self::initialize(connection)
    }

    /// Folds an oversized log back into the database and truncates the file.
    ///
    /// A scan that is killed — SIGKILL, an OOM kill, a killed terminal —
    /// leaves its whole write-ahead log behind, and no later session shrinks
    /// it: the file just grows, one killed run after another, until the
    /// volume fills. Healing at open bounds that to one run's leftovers.
    /// A checkpoint can fail while another connection holds a read snapshot;
    /// the log is then merely large, not corrupt, so the next publish
    /// retries rather than failing the open.
    /// 定位或尝试回收数据库旁的 WAL，不以锁争用判作数据损坏。
    /// 参数：path：数据库原生路径；connection：调用者控制的 SQLite 连接；threshold：WAL 回收阈值，字节。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub(crate) fn heal_oversized_wal(
        path: &Path,
        connection: &Connection,
        threshold: i64,
    ) -> Result<()> {
        let Some(size) = std::fs::metadata(wal_path(path))
            .map(|meta| meta.len())
            .ok()
        else {
            return Ok(());
        };
        if (size as i64) < threshold {
            return Ok(());
        }
        // A checkpoint can fail while another connection holds a read
        // snapshot. The log is then merely large, not corrupt, and the
        // publish path retries: an open must not fail over disk headroom.
        let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        Ok(())
    }

    /// Opens the store, backing up a pre-migration database file first.
    /// Returns the backup path when a migration backup was written; on backup
    /// failure the original database is left untouched.
    /// 打开或初始化连接，遵循各自只读、期限、备份和版本门禁。
    /// 参数：path：数据库原生路径；backup_dir：一致性备份目标目录。
    /// 返回：存储连接及可选一致性备份路径；备份失败不启用新服务。
    pub fn open_with_backup(path: &Path, backup_dir: &Path) -> Result<(Self, Option<PathBuf>)> {
        let probe = Connection::open(path)?;
        let version: i64 = probe.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version >= SUPPORTED_SCHEMA_VERSION || version == 0 {
            return Ok((Self::open(path)?, None));
        }
        std::fs::create_dir_all(backup_dir)?;
        let backup = backup_dir.join(format!(
            "{}.pre-v{SUPPORTED_SCHEMA_VERSION}.bak",
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| StoreError::InvalidGraph("unrepresentable db path".into()))?
        ));
        probe.backup(rusqlite::MAIN_DB, &backup, None)?;
        drop(probe);
        match Self::open(path) {
            Ok(store) => Ok((store, Some(backup))),
            Err(error) => {
                // Keep the database exactly as it was; the backup stays for recovery.
                Err(error)
            }
        }
    }

    /// 打开或初始化连接，遵循各自只读、期限、备份和版本门禁。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：配置和版本校验成功的存储连接，失败不返回可用存储。
    pub fn open_in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    /// 打开或初始化连接，遵循各自只读、期限、备份和版本门禁。
    /// 参数：connection：调用者控制的 SQLite 连接。
    /// 返回：配置和版本校验成功的存储连接，失败不返回可用存储。
    pub(crate) fn initialize(connection: Connection) -> Result<Self> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                connection.execute_batch(V1_SCHEMA)?;
            }
            1..=10 => {}
            other => return Err(StoreError::UnsupportedSchema(other)),
        }
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < 2 {
            migrate_v1_to_v2(&connection)?;
        }
        if version < 3 {
            migrate_v2_to_v3(&connection)?;
        }
        if version < 4 {
            migrate_v3_to_v4(&connection)?;
        }
        if version < 5 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE revision_ownership (revision_id TEXT PRIMARY KEY REFERENCES graph_revisions(revision_id), server_id TEXT NOT NULL, scope_id TEXT NOT NULL); PRAGMA user_version = 5; COMMIT;")?;
        }
        if version < 6 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS node_search (snapshot_id TEXT NOT NULL, id INTEGER NOT NULL, name_fold TEXT NOT NULL, path_fold TEXT NOT NULL, PRIMARY KEY(snapshot_id, id), FOREIGN KEY(snapshot_id, id) REFERENCES nodes(snapshot_id, id) ON DELETE CASCADE); CREATE TABLE IF NOT EXISTS scan_staging_search (job_id TEXT NOT NULL, node_seq INTEGER NOT NULL, name_fold TEXT NOT NULL, path_fold TEXT NOT NULL, PRIMARY KEY(job_id, node_seq));")?;
            {
                let mut query =
                    connection.prepare("SELECT snapshot_id, id, name, locator_key FROM nodes")?;
                let mut rows = query.query([])?;
                while let Some(row) = rows.next()? {
                    let locator: ResourceLocator = from_str(&row.get::<_, String>(3)?)?;
                    let path = match locator {
                        ResourceLocator::NativePath(path) | ResourceLocator::DocumentUri(path) => {
                            path
                        }
                    };
                    connection.execute(
                        "INSERT OR REPLACE INTO node_search VALUES (?1, ?2, ?3, ?4)",
                        params![
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?.to_lowercase(),
                            path.to_lowercase()
                        ],
                    )?;
                }
            }
            connection.execute_batch("CREATE INDEX IF NOT EXISTS nodes_by_name ON nodes(snapshot_id, name, id); PRAGMA user_version = 6; COMMIT;")?;
        }
        if version < 7 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE INDEX IF NOT EXISTS relations_by_source_edge
                     ON relations (snapshot_id, source_entity_id, edge_id);
                 CREATE INDEX IF NOT EXISTS relations_by_target_edge
                     ON relations (snapshot_id, target_entity_id, edge_id);
                 CREATE INDEX IF NOT EXISTS nodes_by_locator_path
                     ON nodes (snapshot_id, json_extract(locator_key, '$.value'), id)
                     WHERE parent_id IS NOT NULL;
                 CREATE INDEX IF NOT EXISTS nodes_by_unknown_parent
                     ON nodes (snapshot_id, parent_id)
                     WHERE NOT (
                         COALESCE(read_error, json_extract(NULLIF(node_json, ''), '$.read_error'), 0) = 0
                         AND COALESCE(json_extract(NULLIF(node_json, ''), '$.size_known'), 1) = 1
                     );
                 PRAGMA user_version = 7;
                 COMMIT;",
            )?;
        }
        if version < 8 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE INDEX IF NOT EXISTS nodes_by_candidate_size
                     ON nodes (snapshot_id, subtree_bytes DESC, id ASC)
                     WHERE (kind = 'directory' OR (kind IS NULL AND json_extract(NULLIF(node_json, ''), '$.kind') = 'directory'))
                       AND subtree_bytes > 0;
                 CREATE INDEX IF NOT EXISTS evidence_by_relation_node
                     ON evidence (snapshot_id, json_extract(evidence_json, '$.relation'), node_id);
                 PRAGMA user_version = 8;
                 COMMIT;",
            )?;
        }
        if version < 9 {
            directory_aggregates::migrate(&connection)?;
        }
        if version < 10 {
            crate::collector_membership_migration::migrate(&connection)?;
        }
        // The locator index cost a quarter of a kilobyte per node and served
        // exactly one query, which nothing on the read path issues. Dropping
        // it is idempotent and needs no schema version: an existing database
        // gets the space back the next time it is opened, and a fresh one
        // never builds it.
        connection.execute_batch("DROP INDEX IF EXISTS nodes_by_locator;")?;
        Ok(Self { connection })
    }
}
