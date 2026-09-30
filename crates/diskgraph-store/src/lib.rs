//! Immutable, per-node SQLite snapshots. No mutation of scanned resources.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use diskgraph_core::{DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, ResourceLocator};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{from_str, to_string};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),
    #[error("revision not found: {0}")]
    RevisionNotFound(String),
    #[error("scope not found: {0}")]
    ScopeNotFound(String),
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("plan not found: {0}")]
    PlanNotFound(String),
    #[error("operation not found: {0}")]
    OperationNotFound(String),
    #[error("recovery entry not found: {0}")]
    RecoveryNotFound(String),
    #[error("approval required: {0}")]
    ApprovalRequired(String),
    #[error("the idempotency key was reused for a different request")]
    IdempotencyConflict,
    #[error("another owner holds this job or resource")]
    StaleOwner,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("retention violation: {0}")]
    RetentionViolation(String),
    #[error("invalid graph: {0}")]
    InvalidGraph(String),
    #[error("value is too large for SQLite INTEGER")]
    IntegerOverflow,
    #[error("unsupported SQLite schema version: {0}")]
    UnsupportedSchema(i64),
}

pub type Result<T> = std::result::Result<T, StoreError>;

mod control;
mod execution;

pub use control::{ControlStore, JobKind, JobRecord, JobState, ScopeRecord};
pub use execution::{
    Approval, IntentState, Operation, OperationItem, OperationItemResult, OperationState, Plan,
    PlanItem, PlanState, RecoveryEntry, RecoveryRule, RecoveryState,
};

/// A store owns only observed metadata. It never opens or removes scanned paths.
pub struct SqliteSnapshotStore {
    connection: Connection,
}

/// The newest schema this build understands; older binaries refuse newer files
/// through [`StoreError::UnsupportedSchema`] (design D6, spec ST-02).
pub const SUPPORTED_SCHEMA_VERSION: i64 = 4;

/// One published graph revision bound to a snapshot.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RevisionRecord {
    pub revision_id: String,
    pub snapshot_id: String,
    pub published_at_unix_ms: u64,
}

/// A WAL this size or larger is a leftover from a killed or out-of-memory
/// run: a healthy scan checkpoints as it goes, and a clean close removes
/// the file, so a session should never open onto one this big.
const WAL_HEAL_THRESHOLD_BYTES: i64 = 64 << 20;

impl SqliteSnapshotStore {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        // WAL + NORMAL: one fsync per checkpoint, not per commit — the
        // atomicity of staging/publish comes from the transaction, not from
        // per-commit fsyncs, and a crash between checkpoints can only lose
        // a not-yet-published staging batch, never a published snapshot.
        // A large read window keeps multi-million-row loads off the page
        // cache cold path.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "mmap_size", 1 << 31)?;
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
    fn heal_oversized_wal(path: &Path, connection: &Connection, threshold: i64) -> Result<()> {
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
    pub fn open_with_backup(path: &Path, backup_dir: &Path) -> Result<(Self, Option<PathBuf>)> {
        let probe = Connection::open(path)?;
        let version: i64 = probe.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        drop(probe);
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
        std::fs::copy(path, &backup)?;
        match Self::open(path) {
            Ok(store) => Ok((store, Some(backup))),
            Err(error) => {
                // Keep the database exactly as it was; the backup stays for recovery.
                Err(error)
            }
        }
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self> {
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                connection.execute_batch(V1_SCHEMA)?;
            }
            1..=4 => {}
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
        // The locator index cost a quarter of a kilobyte per node and served
        // exactly one query, which nothing on the read path issues. Dropping
        // it is idempotent and needs no schema version: an existing database
        // gets the space back the next time it is opened, and a fresh one
        // never builds it.
        connection.execute_batch("DROP INDEX IF EXISTS nodes_by_locator;")?;
        Ok(Self { connection })
    }

    /// Inserts a complete immutable observation atomically; duplicate IDs fail.
    pub fn save(&mut self, graph: &DiskGraph) -> Result<()> {
        validate_graph(graph)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO snapshots (id, root_key, captured_at_unix_ms, snapshot_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                graph.snapshot.id,
                to_string(&graph.snapshot.root)?,
                as_i64(graph.snapshot.captured_at_unix_ms)?,
                to_string(&graph.snapshot)?,
            ],
        )?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO nodes (snapshot_id, id, parent_id, locator_key, name, subtree_bytes,
                 node_json, kind, direct_bytes, files, directories, modified_unix_seconds,
                 file_volume_id, file_id, category_hint, reclaim_hint, read_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )?;
            for node in &graph.nodes {
                let (file_volume_id, file_id) =
                    node.file_identity
                        .as_ref()
                        .map_or((None, None), |identity| {
                            (
                                Some(identity.volume_id.clone()),
                                Some(identity.file_id as i64),
                            )
                        });
                statement.execute(params![
                    graph.snapshot.id,
                    as_i64(node.id)?,
                    node.parent_id.map(as_i64).transpose()?,
                    to_string(&node.locator)?,
                    node.name,
                    as_i64(node.subtree_bytes)?,
                    payload_for(node)?,
                    measured_kind(node),
                    as_i64(node.direct_bytes)?,
                    as_i64(node.files)?,
                    as_i64(node.directories)?,
                    node.modified_unix_seconds,
                    file_volume_id,
                    file_id,
                    node.category_hint,
                    node.reclaim_hint,
                    node.read_error as i64,
                ])?;
            }
        }
        {
            let mut statement = transaction.prepare(
                "INSERT INTO evidence (snapshot_id, node_id, evidence_json)
                 VALUES (?1, ?2, ?3)",
            )?;
            for edge in &graph.evidence {
                statement.execute(params![
                    graph.snapshot.id,
                    as_i64(edge.node_id)?,
                    to_string(edge)?,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn snapshot(&self, id: &str) -> Result<DiskSnapshot> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT snapshot_json FROM snapshots WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        match json {
            Some(json) => Ok(from_str(&json)?),
            None => Err(StoreError::SnapshotNotFound(id.to_owned())),
        }
    }

    pub fn latest_snapshot_id(&self, root: &ResourceLocator) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM snapshots WHERE root_key = ?1
                 ORDER BY captured_at_unix_ms DESC, id DESC LIMIT 1",
                [to_string(root)?],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn load(&self, id: &str) -> Result<DiskGraph> {
        let snapshot = self.snapshot(id)?;
        let nodes = self.load_nodes(id)?;
        let evidence = self.select_json(
            "SELECT evidence_json FROM evidence WHERE snapshot_id = ?1 ORDER BY rowid",
            id,
        )?;
        Ok(DiskGraph {
            snapshot,
            nodes,
            evidence,
        })
    }

    /// The narrow rows a tree view needs. Requires v4 structured columns; a
    /// pre-v4 snapshot yields no rows and the caller falls back to full load.
    pub fn tree_rows(&self, snapshot_id: &str) -> Result<Vec<TreeRow>> {
        let mut statement = self.connection.prepare(
            "SELECT id, parent_id, name, kind, subtree_bytes, direct_bytes,
                    files, directories, read_error, category_hint
             FROM nodes WHERE snapshot_id = ?1 AND kind IS NOT NULL ORDER BY id",
        )?;
        let rows = statement.query_map([snapshot_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                parent_id,
                name,
                kind,
                subtree_bytes,
                direct_bytes,
                files,
                directories,
                read_error,
                category,
            ) = row?;
            Ok((
                id as u64,
                parent_id.map(|value| value as u64),
                name,
                kind,
                subtree_bytes,
                direct_bytes,
                files,
                directories,
                read_error,
                category,
            ))
        })
        .collect()
    }

    /// Loads every node of one snapshot. Pre-v4 rows have NULL structured
    /// columns and fall back to a full JSON parse; v4 rows construct the node
    /// directly, paying one ~50-byte locator parse instead of a ~500-byte
    /// full-node parse (measured: the JSON parse was the dominant cost of a
    /// 4.3M-row load).
    fn load_nodes(&self, snapshot_id: &str) -> Result<Vec<DiskNode>> {
        use rayon::prelude::*;
        type Row = (
            i64,
            Option<i64>,
            String,
            String,
            i64,
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<i64>,
        );
        let mut statement = self.connection.prepare(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes WHERE snapshot_id = ?1 ORDER BY id",
        )?;
        let rows: Vec<Row> = statement
            .query_map([snapshot_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                    row.get(15)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;
        let chunks: Vec<Result<Vec<DiskNode>>> = rows
            .par_chunks(16_384)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|row| -> Result<DiskNode> {
                        let (
                            id,
                            parent_id,
                            locator_key,
                            name,
                            subtree_bytes,
                            node_json,
                            kind,
                            direct_bytes,
                            files,
                            directories,
                            modified_unix_seconds,
                            file_volume_id,
                            file_id,
                            category_hint,
                            reclaim_hint,
                            read_error,
                        ) = row;
                        if let Some(kind) = kind {
                            // v4 fast path: the only remaining text parse is
                            // the locator's small envelope.
                            let locator: ResourceLocator = from_str(locator_key)?;
                            let file_identity = match (file_volume_id, file_id) {
                                (Some(volume_id), Some(id)) => Some(diskgraph_core::FileIdentity {
                                    volume_id: volume_id.clone(),
                                    file_id: *id as u64,
                                }),
                                _ => None,
                            };
                            Ok(DiskNode {
                                id: *id as u64,
                                parent_id: parent_id.map(|value| value as u64),
                                locator,
                                name: name.clone(),
                                kind: kind_from_name(kind)?,
                                subtree_bytes: *subtree_bytes as u64,
                                direct_bytes: direct_bytes.unwrap_or(0) as u64,
                                files: files.unwrap_or(0) as u64,
                                directories: directories.unwrap_or(0) as u64,
                                modified_unix_seconds: *modified_unix_seconds,
                                file_identity,
                                category_hint: category_hint.clone(),
                                reclaim_hint: reclaim_hint.clone(),
                                read_error: read_error.unwrap_or(0) != 0,
                                // Structured rows are only written for fully
                                // measured nodes; unknown sizes stay in JSON.
                                size_known: true,
                            })
                        } else {
                            // Pre-v4 row: parse the archived JSON payload.
                            Ok(from_str(node_json)?)
                        }
                    })
                    .collect()
            })
            .collect();
        let mut nodes = Vec::with_capacity(rows.len());
        for chunk in chunks {
            nodes.append(&mut chunk?);
        }
        Ok(nodes)
    }

    /// The root node of one snapshot (the only node without a parent).
    /// A summary that only needs the subtree total reads this single row
    /// instead of materializing every node.
    pub fn root_node(&self, snapshot_id: &str) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        self.one_node(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes WHERE snapshot_id = ?1 AND parent_id IS NULL LIMIT 1",
            params![snapshot_id],
        )
    }

    pub fn node(&self, snapshot_id: &str, node_id: u64) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        self.one_node(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes WHERE snapshot_id = ?1 AND id = ?2 LIMIT 1",
            params![snapshot_id, as_i64(node_id)?],
        )
    }

    /// Decodes one row of the node column list, or nothing when the row is
    /// absent. Shared by the single-row lookups so a measured node is read
    /// from its columns and only a pre-v4 row falls back to its payload.
    fn one_node<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<Option<DiskNode>> {
        let Some(row) = self
            .connection
            .query_row(sql, params, |row| Ok(NodeRow::from(row)))
            .optional()?
        else {
            return Ok(None);
        };
        Ok(Some(row.into_node()?))
    }

    /// The one child of `parent_id` named `name`, or nothing.
    ///
    /// Unlike `children` this asks for a single entry, so a directory with
    /// ten thousand children costs one index probe instead of a page of
    /// rows the caller would only filter down.
    pub fn child_named(
        &self,
        snapshot_id: &str,
        parent_id: u64,
        name: &str,
    ) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        self.one_node(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes WHERE snapshot_id = ?1 AND parent_id = ?2 AND name = ?3 LIMIT 1",
            params![snapshot_id, as_i64(parent_id)?, name],
        )
    }

    /// Largest immediate children; use `offset` for deterministic paging.
    pub fn children(
        &self,
        snapshot_id: &str,
        parent_id: u64,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<DiskNode>> {
        self.snapshot(snapshot_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes
             WHERE snapshot_id = ?1 AND parent_id = ?2
             ORDER BY subtree_bytes DESC, name ASC, id ASC LIMIT ?3 OFFSET ?4",
        )?;
        let rows = statement.query_map(
            params![
                snapshot_id,
                as_i64(parent_id)?,
                as_i64(limit)?,
                as_i64(offset)?,
            ],
            |row| Ok(NodeRow::from(row)),
        )?;
        rows.map(|row| row?.into_node()).collect()
    }

    pub fn top(&self, snapshot_id: &str, parent_id: u64, limit: u64) -> Result<Vec<DiskNode>> {
        self.children(snapshot_id, parent_id, 0, limit)
    }

    pub fn evidence(&self, snapshot_id: &str, node_id: u64) -> Result<Vec<EvidenceEdge>> {
        self.snapshot(snapshot_id)?;
        let mut statement = self.connection.prepare(
            "SELECT evidence_json FROM evidence
             WHERE snapshot_id = ?1 AND node_id = ?2 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![snapshot_id, as_i64(node_id)?], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| Ok(from_str(&row?)?)).collect()
    }

    /// One node by its exact locator.
    ///
    /// A compatibility lookup, not a query path: no production read uses it,
    /// and without the locator index it scans the snapshot. Anything on a
    /// hot path wants a node id and `revision_layer` instead — an id is
    /// `O(log n)` on the parent index, this is `O(n)`.
    pub fn node_by_locator(
        &self,
        snapshot_id: &str,
        locator: &ResourceLocator,
    ) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        // Read the structured columns, not the archived payload: a measured
        // node no longer carries one, and this must not depend on a row's
        // age. A pre-v4 row still falls back to its payload, which is where
        // its fields live.
        self.connection
            .query_row(
                "SELECT id FROM nodes
                 WHERE snapshot_id = ?1 AND locator_key = ?2 LIMIT 1",
                params![snapshot_id, to_string(locator)?],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .and_then(|id| self.node(snapshot_id, id as u64).transpose())
            .transpose()
    }

    /// Appends scanned nodes to invisible staging for a running job; ordinary
    /// queries never read staging, so a crash before publish exposes nothing.
    pub fn append_staging_nodes(&mut self, job_id: &str, nodes: &[DiskNode]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO scan_staging (job_id, node_seq, node_json) VALUES (?1, ?2, ?3)",
            )?;
            let existing: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(node_seq), 0) FROM scan_staging WHERE job_id = ?1",
                [job_id],
                |row| row.get(0),
            )?;
            for (offset, node) in nodes.iter().enumerate() {
                statement.execute(params![
                    job_id,
                    existing + offset as i64 + 1,
                    to_string(node)?
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Number of staged nodes for one job.
    pub fn staging_node_count(&self, job_id: &str) -> Result<u64> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM scan_staging WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    /// Drops staging for a job without publishing anything.
    pub fn clear_staging(&mut self, job_id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM scan_staging WHERE job_id = ?1", [job_id])?;
        Ok(())
    }

    /// Publishes one scan atomically: snapshot rows, the graph revision, the
    /// per-root latest pointer, and staging cleanup share one transaction
    /// (spec ST-01). A failure leaves the previous latest untouched.
    pub fn publish_revision(
        &mut self,
        job_id: &str,
        graph: &DiskGraph,
        revision_id: &str,
        published_at_unix_ms: u64,
    ) -> Result<()> {
        validate_graph(graph)?;
        let root_key = to_string(&graph.snapshot.root)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO snapshots (id, root_key, captured_at_unix_ms, snapshot_json, pinned)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![
                graph.snapshot.id,
                root_key,
                as_i64(graph.snapshot.captured_at_unix_ms)?,
                to_string(&graph.snapshot)?,
            ],
        )?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO nodes (snapshot_id, id, parent_id, locator_key, name, subtree_bytes,
                 node_json, kind, direct_bytes, files, directories, modified_unix_seconds,
                 file_volume_id, file_id, category_hint, reclaim_hint, read_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )?;
            for node in &graph.nodes {
                let (file_volume_id, file_id) =
                    node.file_identity
                        .as_ref()
                        .map_or((None, None), |identity| {
                            (
                                Some(identity.volume_id.clone()),
                                Some(identity.file_id as i64),
                            )
                        });
                statement.execute(params![
                    graph.snapshot.id,
                    as_i64(node.id)?,
                    node.parent_id.map(as_i64).transpose()?,
                    to_string(&node.locator)?,
                    node.name,
                    as_i64(node.subtree_bytes)?,
                    payload_for(node)?,
                    measured_kind(node),
                    as_i64(node.direct_bytes)?,
                    as_i64(node.files)?,
                    as_i64(node.directories)?,
                    node.modified_unix_seconds,
                    file_volume_id,
                    file_id,
                    node.category_hint,
                    node.reclaim_hint,
                    node.read_error as i64,
                ])?;
            }
            let mut evidence = transaction.prepare(
                "INSERT INTO evidence (snapshot_id, node_id, evidence_json) VALUES (?1, ?2, ?3)",
            )?;
            for edge in &graph.evidence {
                evidence.execute(params![
                    graph.snapshot.id,
                    as_i64(edge.node_id)?,
                    to_string(edge)?,
                ])?;
            }
        }
        transaction.execute(
            "INSERT INTO graph_revisions (revision_id, snapshot_id, published_at_unix_ms)
             VALUES (?1, ?2, ?3)",
            params![
                revision_id,
                graph.snapshot.id,
                as_i64(published_at_unix_ms)?
            ],
        )?;
        transaction.execute(
            "INSERT INTO latest_revision (root_key, revision_id) VALUES (?1, ?2)
             ON CONFLICT(root_key) DO UPDATE SET revision_id = ?2",
            params![root_key, revision_id],
        )?;
        transaction.execute("DELETE FROM scan_staging WHERE job_id = ?1", [job_id])?;
        transaction.commit()?;
        // The scan's entire write-ahead log is now redundant. Folding it back
        // here — rather than waiting for the next checkpoint — is what keeps a
        // multi-million-node publish from leaving a log nearly as large as the
        // snapshot it just wrote. A concurrent reader can hold a snapshot open
        // and make the checkpoint a no-op; the log is then merely large, and
        // the next open heals it.
        let _ = self
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        Ok(())
    }

    /// One published revision.
    pub fn revision(&self, revision_id: &str) -> Result<RevisionRecord> {
        self.connection
            .query_row(
                "SELECT snapshot_id, published_at_unix_ms FROM graph_revisions WHERE revision_id = ?1",
                [revision_id],
                |row| {
                    Ok(RevisionRecord {
                        revision_id: revision_id.to_owned(),
                        snapshot_id: row.get(0)?,
                        published_at_unix_ms: row.get::<_, i64>(1)?.try_into().unwrap_or(0),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::RevisionNotFound(revision_id.to_owned()))
    }

    /// The latest published revision for a root locator, if any.
    pub fn latest_revision_for_root(&self, root: &ResourceLocator) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT lr.revision_id FROM latest_revision lr WHERE lr.root_key = ?1",
                [to_string(root)?],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Loads the full v1 graph behind one revision (v1 projections preserved).
    pub fn load_revision(&self, revision_id: &str) -> Result<DiskGraph> {
        self.load(&self.revision(revision_id)?.snapshot_id)
    }

    /// Marks or clears the retention pin on a snapshot.
    pub fn pin_snapshot(&self, snapshot_id: &str, pinned: bool) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE snapshots SET pinned = ?2 WHERE id = ?1",
            params![snapshot_id, pinned as i64],
        )?;
        if changed == 0 {
            return Err(StoreError::SnapshotNotFound(snapshot_id.to_owned()));
        }
        Ok(())
    }

    /// Whether a snapshot is pinned against retention.
    pub fn snapshot_pinned(&self, snapshot_id: &str) -> Result<bool> {
        self.snapshot(snapshot_id)?;
        let pinned: i64 = self.connection.query_row(
            "SELECT pinned FROM snapshots WHERE id = ?1",
            [snapshot_id],
            |row| row.get(0),
        )?;
        Ok(pinned != 0)
    }

    /// Removes a snapshot from graph history. Refused when pinned, referenced
    /// by a published revision, or backing the current latest pointer
    /// (spec ST-04); never touches user files or control data.
    pub fn remove_snapshot(&mut self, snapshot_id: &str) -> Result<()> {
        if self.snapshot_pinned(snapshot_id)? {
            return Err(StoreError::RetentionViolation(format!(
                "snapshot {snapshot_id} is pinned"
            )));
        }
        let references: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM graph_revisions WHERE snapshot_id = ?1",
            [snapshot_id],
            |row| row.get(0),
        )?;
        if references > 0 {
            return Err(StoreError::RetentionViolation(format!(
                "snapshot {snapshot_id} backs {references} published revision(s)"
            )));
        }
        let changed = self
            .connection
            .execute("DELETE FROM snapshots WHERE id = ?1", [snapshot_id])?;
        if changed == 0 {
            return Err(StoreError::SnapshotNotFound(snapshot_id.to_owned()));
        }
        Ok(())
    }

    /// Lists snapshots (optionally for one root), newest first, paged.
    pub fn list_snapshots(
        &self,
        root: Option<&ResourceLocator>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<DiskSnapshot>> {
        let (sql, key): (&str, Option<String>) = match root {
            Some(root) => (
                "SELECT snapshot_json FROM snapshots WHERE root_key = ?1
                 ORDER BY captured_at_unix_ms DESC, id DESC LIMIT ?2 OFFSET ?3",
                Some(to_string(root)?),
            ),
            None => (
                "SELECT snapshot_json FROM snapshots
                 ORDER BY captured_at_unix_ms DESC, id DESC LIMIT ?1 OFFSET ?2",
                None,
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let map_row = |row: &rusqlite::Row<'_>| row.get::<_, String>(0);
        let rows = match key {
            Some(key) => {
                statement.query_map(params![key, as_i64(limit)?, as_i64(offset)?], map_row)?
            }
            None => statement.query_map(params![as_i64(limit)?, as_i64(offset)?], map_row)?,
        };
        rows.map(|row| Ok(from_str::<DiskSnapshot>(&row?)?))
            .collect()
    }

    /// Records one collector batch: run, entities, evidence, and validated
    /// typed edges, atomically for one snapshot (EV-01/EV-02). Endpoint
    /// violations reject the whole batch before anything is written.
    pub fn record_collector_batch(
        &mut self,
        snapshot_id: &str,
        run: &diskgraph_core::CollectorRun,
        entities: &[diskgraph_core::Entity],
        evidence: &[diskgraph_core::EvidenceRecord],
        edges: &[diskgraph_core::RelationEdge],
    ) -> Result<()> {
        self.snapshot(snapshot_id)?;
        if run.snapshot_id != snapshot_id {
            return Err(StoreError::InvalidGraph(
                "collector run targets a different snapshot".into(),
            ));
        }
        let entity_kinds: Vec<(String, diskgraph_core::EntityKind)> = entities
            .iter()
            .map(|entity| (entity.entity_id.clone(), entity.kind))
            .collect();
        diskgraph_core::validate_edges(edges, &entity_kinds).map_err(|error| {
            StoreError::InvalidGraph(format!(
                "edge {} ({}): {}",
                error.edge_id,
                error.relation.wire_name(),
                error.reason
            ))
        })?;
        if edges.iter().any(|edge| {
            edge.evidence_refs.iter().any(|(evidence_id, _)| {
                !evidence
                    .iter()
                    .any(|record| record.evidence_id == *evidence_id)
            })
        }) {
            return Err(StoreError::InvalidGraph(
                "edge references evidence outside this batch".into(),
            ));
        }
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO collector_runs (run_id, snapshot_id, collector_id, collector_version, rule_version, observed_at_unix_ms, coverage_complete, run_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                run.run_id,
                snapshot_id,
                run.collector_id,
                run.collector_version as i64,
                run.rule_version as i64,
                as_i64(run.observed_at_unix_ms)?,
                run.coverage_complete as i64,
                to_string(run)?,
            ],
        )?;
        {
            let mut entity_statement = transaction.prepare(
                "INSERT INTO entities (snapshot_id, entity_id, kind, entity_json) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for entity in entities {
                entity_statement.execute(params![
                    snapshot_id,
                    entity.entity_id,
                    entity.kind.wire_name(),
                    to_string(entity)?,
                ])?;
            }
            let mut evidence_statement = transaction.prepare(
                "INSERT INTO evidence_records (snapshot_id, evidence_id, run_id, evidence_json) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for record in evidence {
                evidence_statement.execute(params![
                    snapshot_id,
                    record.evidence_id,
                    record.run_id,
                    to_string(record)?,
                ])?;
            }
            let mut edge_statement = transaction.prepare(
                "INSERT INTO relations (snapshot_id, edge_id, source_entity_id, relation, target_entity_id, edge_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for edge in edges {
                edge_statement.execute(params![
                    snapshot_id,
                    edge.edge_id,
                    edge.source_entity_id,
                    edge.relation.wire_name(),
                    edge.target_entity_id,
                    to_string(edge)?,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Binds collector runs to a published revision with their roles
    /// (active vs dependency-only) (EV-05).
    pub fn bind_runs_to_revision(
        &mut self,
        revision_id: &str,
        runs: &[(&str, &str)],
    ) -> Result<()> {
        self.revision(revision_id)?;
        let transaction = self.connection.transaction()?;
        {
            let mut statement = transaction.prepare(
                "INSERT OR IGNORE INTO revision_runs (revision_id, run_id, role) VALUES (?1, ?2, ?3)",
            )?;
            for (run_id, role) in runs {
                statement.execute(params![revision_id, run_id, role])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Outgoing edges of one entity, optionally filtered by relation.
    pub fn edges_from(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
    ) -> Result<Vec<diskgraph_core::RelationEdge>> {
        self.select_edges(snapshot_id, "source_entity_id", entity_id, relation)
    }

    /// Incoming edges of one entity, optionally filtered by relation.
    pub fn edges_to(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
    ) -> Result<Vec<diskgraph_core::RelationEdge>> {
        self.select_edges(snapshot_id, "target_entity_id", entity_id, relation)
    }

    fn select_edges(
        &self,
        snapshot_id: &str,
        side: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
    ) -> Result<Vec<diskgraph_core::RelationEdge>> {
        let sql = match relation {
            Some(_) => format!(
                "SELECT edge_json FROM relations
                 WHERE snapshot_id = ?1 AND {side} = ?2 AND relation = ?3 ORDER BY edge_id"
            ),
            None => format!(
                "SELECT edge_json FROM relations
                 WHERE snapshot_id = ?1 AND {side} = ?2 ORDER BY edge_id"
            ),
        };
        let mut statement = self.connection.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| row.get::<_, String>(0);
        let rows = match relation {
            Some(relation) => statement.query_map(
                params![snapshot_id, entity_id, relation.wire_name()],
                map_row,
            )?,
            None => statement.query_map(params![snapshot_id, entity_id], map_row)?,
        };
        let edges: Vec<diskgraph_core::RelationEdge> = rows
            .map(|row| Ok(from_str::<diskgraph_core::RelationEdge>(&row?)?))
            .collect::<Result<Vec<_>>>()?;
        Ok(edges)
    }

    /// Every typed edge of one snapshot (bounded by the caller).
    pub fn all_edges(&self, snapshot_id: &str) -> Result<Vec<diskgraph_core::RelationEdge>> {
        let mut statement = self
            .connection
            .prepare("SELECT edge_json FROM relations WHERE snapshot_id = ?1 ORDER BY edge_id")?;
        let rows = statement.query_map([snapshot_id], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(from_str(&row?)?)).collect()
    }

    /// Loads one entity of a snapshot.
    pub fn entity(
        &self,
        snapshot_id: &str,
        entity_id: &str,
    ) -> Result<Option<diskgraph_core::Entity>> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT entity_json FROM entities WHERE snapshot_id = ?1 AND entity_id = ?2",
                params![snapshot_id, entity_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    /// Evidence records referenced by the given edges (explain's provenance).
    pub fn evidence_for_edges(
        &self,
        snapshot_id: &str,
        edge_ids: &[String],
    ) -> Result<Vec<diskgraph_core::EvidenceRecord>> {
        let mut records = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for edge_id in edge_ids {
            let edge_json: String = self
                .connection
                .query_row(
                    "SELECT edge_json FROM relations WHERE snapshot_id = ?1 AND edge_id = ?2",
                    params![snapshot_id, edge_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| StoreError::InvalidGraph(format!("missing edge {edge_id}")))?;
            let edge: diskgraph_core::RelationEdge = from_str(&edge_json)?;
            for (evidence_id, _) in edge.evidence_refs {
                let json: Option<String> = self
                    .connection
                    .query_row(
                        "SELECT evidence_json FROM evidence_records
                     WHERE snapshot_id = ?1 AND evidence_id = ?2",
                        params![snapshot_id, evidence_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(json) = json {
                    let record: diskgraph_core::EvidenceRecord = from_str(&json)?;
                    if seen.insert(record.evidence_id.clone()) {
                        records.push(record);
                    }
                }
            }
        }
        Ok(records)
    }

    fn select_json<T: serde::de::DeserializeOwned + Send>(
        &self,
        sql: &str,
        snapshot_id: &str,
    ) -> Result<Vec<T>> {
        let mut statement = self.connection.prepare(sql)?;
        // Vec<u8> skips the per-row UTF-8 validation and String allocation
        // (measured at ~30s of a 91s multi-million-row load); SIMD parsing
        // then keeps the same serde contract at a fraction of the cost.
        let rows = statement.query_map([snapshot_id], |row| {
            Ok(row.get_ref(0)?.as_bytes()?.to_vec())
        })?;
        // Multi-million-row loads spend most of their budget in JSON parsing
        // (measured on the real 4.3M-row workload): simd-json is ~2x SLOWER
        // than serde_json per call at this document size, so the parser stays
        // serde_json. Two real wins instead: skip per-row UTF-8 validation
        // and String allocation with as_bytes(), and parallelize the CPU-bound
        // parsing across cores in bounded chunks (each chunk keeps insertion
        // order, so the result is identical to sequential).
        use rayon::prelude::*;
        let raw: Vec<Vec<u8>> = rows.collect::<std::result::Result<_, _>>()?;
        let chunks: Vec<Result<Vec<T>>> = raw
            .par_chunks(16_384)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|bytes| serde_json::from_slice(bytes).map_err(StoreError::from))
                    .collect()
            })
            .collect();
        let mut out = Vec::with_capacity(raw.len());
        for chunk in chunks {
            out.append(&mut chunk?);
        }
        Ok(out)
    }
}

/// NodeKind's stable snake-case wire name (must match serde's rendering).
fn kind_name(kind: diskgraph_core::NodeKind) -> &'static str {
    match kind {
        diskgraph_core::NodeKind::Directory => "directory",
        diskgraph_core::NodeKind::File => "file",
        diskgraph_core::NodeKind::Symlink => "symlink",
        diskgraph_core::NodeKind::Other => "other",
    }
}

/// Parses the wire name written by kind_name; unknown values are corrupt rows.
fn kind_from_name(name: &str) -> Result<diskgraph_core::NodeKind> {
    Ok(match name {
        "directory" => diskgraph_core::NodeKind::Directory,
        "file" => diskgraph_core::NodeKind::File,
        "symlink" => diskgraph_core::NodeKind::Symlink,
        "other" => diskgraph_core::NodeKind::Other,
        _ => {
            return Err(StoreError::InvalidGraph(format!(
                "unknown node kind: {name}"
            )));
        }
    })
}

/// One narrow node row for the tree view: no locator, no JSON payload.
pub type TreeRow = (
    u64,
    Option<u64>,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<String>,
);

fn as_i64(value: u64) -> Result<i64> {
    value.try_into().map_err(|_| StoreError::IntegerOverflow)
}

/// The write-ahead log beside a database file, named the way SQLite names
/// it: the database path with a `-wal` suffix.
fn wal_path(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_os_string();
    name.push("-wal");
    PathBuf::from(name)
}

/// The archived JSON payload for one node row.
///
/// Every measured node also has its fields in the structured columns, and
/// the read path reconstructs it from those — it never parses the payload
/// for such a row. Writing it anyway cost half a kilobyte per node, so a
/// four-million-node index carried two and a half gigabytes of a copy of
/// data already in the row. Only a node whose size is unknown still has the
/// payload as its record, and that is the one case that gets one.
fn payload_for(node: &DiskNode) -> Result<String> {
    if node.size_known {
        Ok(String::new())
    } else {
        Ok(to_string(node)?)
    }
}

/// The `kind` column doubles as the marker for a fully measured row: the read
/// path reconstructs a node from the columns when it is set, and falls back to
/// the archived payload when it is not.
///
/// A node whose size could not be measured therefore leaves it empty. Filling
/// it anyway would read the node back as measured — "unknown" silently
/// becoming "zero bytes", which is the one thing a size report must never
/// do to a directory it could not open.
fn measured_kind(node: &DiskNode) -> Option<&'static str> {
    if node.size_known {
        Some(kind_name(node.kind))
    } else {
        None
    }
}

/// One row of a node query, in whichever of the two shapes it was stored:
/// a measured node with its fields in columns, or a pre-v4 node whose
/// fields live only in the archived payload.
struct NodeRow {
    id: i64,
    parent_id: Option<i64>,
    locator_key: String,
    name: String,
    subtree_bytes: i64,
    node_json: String,
    kind: Option<String>,
    direct_bytes: Option<i64>,
    files: Option<i64>,
    directories: Option<i64>,
    modified_unix_seconds: Option<i64>,
    file_volume_id: Option<String>,
    file_id: Option<i64>,
    category_hint: Option<String>,
    reclaim_hint: Option<String>,
    read_error: Option<i64>,
}

impl<'row> From<&'row rusqlite::Row<'row>> for NodeRow {
    fn from(row: &'row rusqlite::Row<'row>) -> Self {
        Self {
            id: row.get(0).unwrap_or_default(),
            parent_id: row.get(1).unwrap_or_default(),
            locator_key: row.get(2).unwrap_or_default(),
            name: row.get(3).unwrap_or_default(),
            subtree_bytes: row.get(4).unwrap_or_default(),
            node_json: row.get(5).unwrap_or_default(),
            kind: row.get(6).unwrap_or_default(),
            direct_bytes: row.get(7).unwrap_or_default(),
            files: row.get(8).unwrap_or_default(),
            directories: row.get(9).unwrap_or_default(),
            modified_unix_seconds: row.get(10).unwrap_or_default(),
            file_volume_id: row.get(11).unwrap_or_default(),
            file_id: row.get(12).unwrap_or_default(),
            category_hint: row.get(13).unwrap_or_default(),
            reclaim_hint: row.get(14).unwrap_or_default(),
            read_error: row.get(15).unwrap_or_default(),
        }
    }
}

impl NodeRow {
    /// The node this row describes. A row with a structured `kind` is read
    /// from its columns; a pre-v4 row falls back to the payload, which is
    /// where its fields live.
    fn into_node(self) -> Result<DiskNode> {
        let Some(kind) = self.kind else {
            return Ok(from_str(&self.node_json)?);
        };
        let locator: ResourceLocator = from_str(&self.locator_key)?;
        let file_identity = match (self.file_volume_id, self.file_id) {
            (Some(volume_id), Some(id)) => Some(diskgraph_core::FileIdentity {
                volume_id,
                file_id: id as u64,
            }),
            _ => None,
        };
        Ok(DiskNode {
            id: self.id as u64,
            parent_id: self.parent_id.map(|value| value as u64),
            locator,
            name: self.name,
            kind: kind_from_name(&kind)?,
            subtree_bytes: self.subtree_bytes as u64,
            direct_bytes: self.direct_bytes.unwrap_or(0) as u64,
            files: self.files.unwrap_or(0) as u64,
            directories: self.directories.unwrap_or(0) as u64,
            modified_unix_seconds: self.modified_unix_seconds,
            file_identity,
            category_hint: self.category_hint,
            reclaim_hint: self.reclaim_hint,
            read_error: self.read_error.unwrap_or(0) != 0,
            // Structured rows are only written for fully measured nodes;
            // unknown sizes stay in the payload.
            size_known: true,
        })
    }
}

/// The v1 schema, verbatim: fresh databases are created at v1 and then
/// migrated forward, so v1 rows never skip the migration path (ST-02).
const V1_SCHEMA: &str = "PRAGMA foreign_keys = ON;
     BEGIN IMMEDIATE;
     CREATE TABLE IF NOT EXISTS snapshots (
         id TEXT PRIMARY KEY,
         root_key TEXT NOT NULL,
         captured_at_unix_ms INTEGER NOT NULL,
         snapshot_json TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS snapshots_by_root_time
         ON snapshots (root_key, captured_at_unix_ms DESC, id DESC);
     CREATE TABLE IF NOT EXISTS nodes (
         snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
         id INTEGER NOT NULL,
         parent_id INTEGER,
         locator_key TEXT NOT NULL,
         name TEXT NOT NULL,
         subtree_bytes INTEGER NOT NULL,
         node_json TEXT NOT NULL,
         PRIMARY KEY (snapshot_id, id)
     );
     CREATE INDEX IF NOT EXISTS nodes_by_parent_size
         ON nodes (snapshot_id, parent_id, subtree_bytes DESC, name ASC);
     CREATE TABLE IF NOT EXISTS evidence (
         snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
         node_id INTEGER NOT NULL,
         evidence_json TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS evidence_by_node
         ON evidence (snapshot_id, node_id);
     PRAGMA user_version = 1;
     COMMIT;";

/// Adds the v2 layer (revisions, latest pointers, staging, retention pin) in
/// one transaction; a failed migration rolls back and the v1 file stays v1.
fn migrate_v1_to_v2(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "ALTER TABLE snapshots ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
         CREATE TABLE graph_revisions (
             revision_id TEXT PRIMARY KEY,
             snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
             published_at_unix_ms INTEGER NOT NULL
         );
         CREATE INDEX revisions_by_snapshot
             ON graph_revisions (snapshot_id, published_at_unix_ms DESC);
         CREATE TABLE latest_revision (
             root_key TEXT PRIMARY KEY,
             revision_id TEXT NOT NULL REFERENCES graph_revisions(revision_id)
         );
         CREATE TABLE scan_staging (
             job_id TEXT NOT NULL,
             node_seq INTEGER NOT NULL,
             node_json TEXT NOT NULL,
             PRIMARY KEY (job_id, node_seq)
         );
         PRAGMA user_version = 2;",
    )?;
    transaction.commit()?;
    Ok(())
}

/// Adds the evidence layer: collector runs, typed entities and relations with
/// provenance, and the revision-to-run binding (EV-01..EV-05).
fn migrate_v2_to_v3(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "CREATE TABLE collector_runs (
             run_id TEXT PRIMARY KEY,
             snapshot_id TEXT NOT NULL,
             collector_id TEXT NOT NULL,
             collector_version INTEGER NOT NULL,
             rule_version INTEGER NOT NULL,
             observed_at_unix_ms INTEGER NOT NULL,
             coverage_complete INTEGER NOT NULL,
             run_json TEXT NOT NULL
         );
         CREATE INDEX runs_by_snapshot
             ON collector_runs (snapshot_id, observed_at_unix_ms DESC);
         CREATE TABLE entities (
             snapshot_id TEXT NOT NULL,
             entity_id TEXT NOT NULL,
             kind TEXT NOT NULL,
             entity_json TEXT NOT NULL,
             PRIMARY KEY (snapshot_id, entity_id)
         );
         CREATE TABLE relations (
             snapshot_id TEXT NOT NULL,
             edge_id TEXT NOT NULL,
             source_entity_id TEXT NOT NULL,
             relation TEXT NOT NULL,
             target_entity_id TEXT NOT NULL,
             edge_json TEXT NOT NULL,
             PRIMARY KEY (snapshot_id, edge_id)
         );
         CREATE INDEX relations_by_source
             ON relations (snapshot_id, source_entity_id, relation);
         CREATE INDEX relations_by_target
             ON relations (snapshot_id, target_entity_id, relation);
         CREATE TABLE evidence_records (
             snapshot_id TEXT NOT NULL,
             evidence_id TEXT NOT NULL,
             run_id TEXT NOT NULL,
             evidence_json TEXT NOT NULL,
             PRIMARY KEY (snapshot_id, evidence_id)
         );
         CREATE INDEX evidence_by_run
             ON evidence_records (snapshot_id, run_id);
         CREATE TABLE revision_runs (
             revision_id TEXT NOT NULL REFERENCES graph_revisions(revision_id),
             run_id TEXT NOT NULL REFERENCES collector_runs(run_id),
             role TEXT NOT NULL,
             PRIMARY KEY (revision_id, run_id)
         );
         PRAGMA user_version = 3;",
    )?;
    transaction.commit()?;
    Ok(())
}

/// v3 -> v4: nodes gain structured columns mirroring node_json, so
/// multi-million-row loads stop paying a full JSON parse per row. Pre-v4
/// rows keep NULL in these columns and take the JSON fallback at read time.
fn migrate_v3_to_v4(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "ALTER TABLE nodes ADD COLUMN kind TEXT;
         ALTER TABLE nodes ADD COLUMN direct_bytes INTEGER;
         ALTER TABLE nodes ADD COLUMN files INTEGER;
         ALTER TABLE nodes ADD COLUMN directories INTEGER;
         ALTER TABLE nodes ADD COLUMN modified_unix_seconds INTEGER;
         ALTER TABLE nodes ADD COLUMN file_volume_id TEXT;
         ALTER TABLE nodes ADD COLUMN file_id INTEGER;
         ALTER TABLE nodes ADD COLUMN category_hint TEXT;
         ALTER TABLE nodes ADD COLUMN reclaim_hint TEXT;
         ALTER TABLE nodes ADD COLUMN read_error INTEGER;
         PRAGMA user_version = 4;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn validate_graph(graph: &DiskGraph) -> Result<()> {
    if graph.snapshot.id.is_empty() || graph.nodes.is_empty() {
        return Err(StoreError::InvalidGraph(
            "missing snapshot ID or nodes".into(),
        ));
    }
    let ids: HashSet<_> = graph.nodes.iter().map(|node| node.id).collect();
    if ids.len() != graph.nodes.len() {
        return Err(StoreError::InvalidGraph("duplicate node IDs".into()));
    }
    if graph
        .nodes
        .iter()
        .any(|node| node.parent_id.is_some_and(|parent| !ids.contains(&parent)))
    {
        return Err(StoreError::InvalidGraph("missing parent node".into()));
    }
    let roots: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.parent_id.is_none())
        .collect();
    if roots.len() != 1 || roots[0].locator != graph.snapshot.root {
        return Err(StoreError::InvalidGraph(
            "snapshot must have one matching root".into(),
        ));
    }
    let locators: HashSet<_> = graph.nodes.iter().map(|node| &node.locator).collect();
    if locators.len() != graph.nodes.len() {
        return Err(StoreError::InvalidGraph(
            "duplicate resource locators".into(),
        ));
    }
    if graph.snapshot.coverage.complete
        && (graph.snapshot.coverage.unreadable_nodes > 0 || graph.snapshot.coverage.depth_limited)
    {
        return Err(StoreError::InvalidGraph(
            "inconsistent scan coverage".into(),
        ));
    }
    if graph
        .evidence
        .iter()
        .any(|edge| !ids.contains(&edge.node_id) || edge.confidence > 100)
    {
        return Err(StoreError::InvalidGraph("invalid evidence".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use diskgraph_core::{
        DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, NodeKind, ScanCoverage,
        ScanSettings,
    };

    use super::*;

    fn graph(id: &str, bytes: u64) -> DiskGraph {
        let root = ResourceLocator::NativePath("/tmp/diskgraph-test".into());
        DiskGraph {
            snapshot: DiskSnapshot {
                id: id.into(),
                root: root.clone(),
                volume_id: Some("test-volume".into()),
                captured_at_unix_ms: bytes,
                settings: ScanSettings {
                    apparent_size: false,
                    follow_links: false,
                    include_hidden: true,
                    one_filesystem: true,
                    max_depth: None,
                    dedup_hardlinks: true,
                },
                coverage: ScanCoverage {
                    complete: true,
                    unreadable_nodes: 0,
                    depth_limited: false,
                },
            },
            nodes: vec![
                DiskNode {
                    id: 1,
                    parent_id: None,
                    locator: root,
                    name: "diskgraph-test".into(),
                    kind: NodeKind::Directory,
                    subtree_bytes: bytes,
                    size_known: true,
                    direct_bytes: 0,
                    files: 1,
                    directories: 1,
                    modified_unix_seconds: None,
                    file_identity: None,
                    category_hint: None,
                    reclaim_hint: None,
                    read_error: false,
                },
                DiskNode {
                    id: 2,
                    parent_id: Some(1),
                    locator: ResourceLocator::NativePath("/tmp/diskgraph-test/cache".into()),
                    name: "cache".into(),
                    kind: NodeKind::Directory,
                    subtree_bytes: bytes,
                    size_known: true,
                    direct_bytes: bytes,
                    files: 1,
                    directories: 1,
                    modified_unix_seconds: None,
                    file_identity: None,
                    category_hint: None,
                    reclaim_hint: None,
                    read_error: false,
                },
            ],
            evidence: vec![EvidenceEdge {
                node_id: 2,
                relation: EvidenceRelation::Rebuildable,
                subject: "test".into(),
                source: "unit-test".into(),
                observed_at_unix_ms: bytes,
                confidence: 80,
            }],
        }
    }

    #[test]
    fn snapshots_are_immutable_and_queries_are_ordered() {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let first = graph("one", 100);
        let second = graph("two", 150);
        store.save(&first).unwrap();
        store.save(&second).unwrap();
        assert!(store.save(&first).is_err());
        assert_eq!(store.load("one").unwrap(), first);
        assert_eq!(
            store.latest_snapshot_id(&first.snapshot.root).unwrap(),
            Some("two".into())
        );
        assert_eq!(store.top("two", 1, 1).unwrap()[0].id, 2);
        assert!(store.children("two", 1, 1, 1).unwrap().is_empty());
        assert_eq!(store.evidence("two", 2).unwrap().len(), 1);
        assert_eq!(
            store
                .node_by_locator("two", &second.nodes[1].locator)
                .unwrap()
                .unwrap()
                .subtree_bytes,
            150
        );
    }

    #[test]
    fn rejects_invalid_graph_without_persisting_a_partial_snapshot() {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let mut invalid = graph("invalid", 100);
        invalid.evidence[0].node_id = 999;
        assert!(store.save(&invalid).is_err());
        assert!(store.snapshot("invalid").is_err());
    }

    /// The write-ahead log beside a database, as SQLite writes it.
    fn wal_len(path: &Path) -> u64 {
        std::fs::metadata(wal_path(path))
            .map(|meta| meta.len())
            .unwrap_or(0)
    }

    #[test]
    fn a_measured_node_stores_no_payload_and_still_reads_back_identically() {
        let dir = std::env::temp_dir().join(format!("diskgraph-payload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diskgraph.sqlite");
        let original = graph("slim", 4_096);
        {
            let mut store = SqliteSnapshotStore::open(&path).unwrap();
            store
                .publish_revision("job", &original, "rev-0", 1_700_000_000)
                .unwrap();
        }
        let connection = Connection::open(&path).unwrap();
        let stored: String = connection
            .query_row(
                "SELECT node_json FROM nodes WHERE snapshot_id = 'slim' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            stored.is_empty(),
            "a measured row must not repeat itself: {stored}"
        );
        // Every reader goes through the columns now, and they must return the
        // same node the writer was handed: the payload was a copy, and a
        // copy that drifts is worse than no copy.
        let store = SqliteSnapshotStore::open(&path).unwrap();
        assert_eq!(store.load("slim").unwrap(), original);
        assert_eq!(store.node("slim", 2).unwrap().unwrap(), original.nodes[1]);
        assert_eq!(store.root_node("slim").unwrap().unwrap(), original.nodes[0]);
        assert_eq!(
            store.children("slim", 1, 0, 10).unwrap(),
            vec![original.nodes[1].clone()]
        );
        assert_eq!(
            store
                .node_by_locator("slim", &original.nodes[1].locator)
                .unwrap()
                .unwrap(),
            original.nodes[1]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_node_of_unknown_size_keeps_its_payload_because_it_is_the_record() {
        let dir = std::env::temp_dir().join(format!("diskgraph-unknown-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diskgraph.sqlite");
        // A node whose size could not be measured has its fields nowhere
        // else: the columns are only written for fully measured nodes, so
        // dropping its payload would erase it.
        let mut unknown = graph("partial", 100);
        unknown.nodes[1].size_known = false;
        {
            let mut store = SqliteSnapshotStore::open(&path).unwrap();
            store
                .publish_revision("job", &unknown, "rev-0", 1_700_000_000)
                .unwrap();
        }
        let store = SqliteSnapshotStore::open(&path).unwrap();
        assert_eq!(store.node("partial", 2).unwrap().unwrap(), unknown.nodes[1]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_locator_index_is_not_rebuilt() {
        let dir = std::env::temp_dir().join(format!("diskgraph-noindex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diskgraph.sqlite");
        {
            let mut store = SqliteSnapshotStore::open(&path).unwrap();
            store
                .publish_revision("job", &graph("slim", 100), "rev-0", 1_700_000_000)
                .unwrap();
        }
        // Reopened, so the open-time drop runs against a populated database.
        let store = SqliteSnapshotStore::open(&path).unwrap();
        let count = |name: &str| -> i64 {
            store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'index' AND name = ?1",
                    [name],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(
            count("nodes_by_locator"),
            0,
            "the index cost a quarter kilobyte a node and served no read path"
        );
        // The parent index is what queries actually use, and it stays.
        assert_eq!(count("nodes_by_parent_size"), 1);
        assert_eq!(store.children("slim", 1, 0, 10).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_oversized_wal_is_folded_back_into_the_database_at_open() {
        let dir = std::env::temp_dir().join(format!("diskgraph-wal-heal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diskgraph.sqlite");
        {
            let mut store = SqliteSnapshotStore::open(&path).unwrap();
            store
                .publish_revision("job", &graph("one", 100), "rev-0", 1_700_000_000)
                .unwrap();
        }
        // A run that is killed before its log is folded back: the log is a
        // genuine one from a genuine write, left behind because the process
        // never got to the end of its scan.
        let killed = Connection::open(&path).unwrap();
        killed.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        killed
            .execute_batch("CREATE TABLE IF NOT EXISTS fill (payload BLOB);")
            .unwrap();
        for _ in 0..4_000 {
            killed
                .execute("INSERT INTO fill VALUES (zeroblob(1024))", [])
                .unwrap();
        }
        let before = wal_len(&path);
        assert!(
            before > 1 << 20,
            "the killed run must leave megabytes behind, not {before} bytes"
        );
        // Leaked rather than dropped: a closing connection checkpoints and
        // deletes the log, which is the case the heal exists for the *other*
        // side of. A SIGKILL is what leaves the file on disk.
        std::mem::forget(killed);

        // A threshold of one byte is what makes the heal observable: the
        // production threshold is far above any healthy log.
        let connection = Connection::open(&path).unwrap();
        SqliteSnapshotStore::heal_oversized_wal(&path, &connection, 1).unwrap();
        let after = wal_len(&path);
        assert!(after < before, "the log must shrink, not just be checked");
        assert!(after <= WAL_HEAL_THRESHOLD_BYTES as u64);
        // The data the log carried is still in the database: healing is
        // housekeeping, never a rollback.
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM fill", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 4_000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_healthy_log_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("diskgraph-wal-quiet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diskgraph.sqlite");
        {
            let mut store = SqliteSnapshotStore::open(&path).unwrap();
            store
                .publish_revision("job", &graph("only", 100), "rev-0", 1_700_000_000)
                .unwrap();
        }
        let connection = Connection::open(&path).unwrap();
        // Below the threshold nothing is touched: a checkpoint would rewrite
        // pages for no reason on every open.
        let before = wal_len(&path);
        SqliteSnapshotStore::heal_oversized_wal(&path, &connection, i64::MAX).unwrap();
        assert_eq!(wal_len(&path), before);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_publish_leaves_no_write_ahead_log_behind() {
        let dir =
            std::env::temp_dir().join(format!("diskgraph-publish-wal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diskgraph.sqlite");
        let mut store = SqliteSnapshotStore::open(&path).unwrap();
        store
            .publish_revision("job", &graph("one", 100), "rev-0", 1_700_000_000)
            .unwrap();
        // The whole log of the scan is redundant the moment the transaction
        // commits; leaving it on disk is what made a four-million-node scan
        // cost nearly twice its snapshot size.
        assert_eq!(
            wal_len(&path),
            0,
            "publishing must fold its log back into the database"
        );
        // And the snapshot is still readable: the checkpoint is housekeeping,
        // not a second commit.
        assert_eq!(store.load("one").unwrap().nodes.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_wal_is_capped_so_it_cannot_grow_without_bound() {
        let dir = std::env::temp_dir().join(format!("diskgraph-wal-limit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diskgraph.sqlite");
        let store = SqliteSnapshotStore::open(&path).unwrap();
        let limit: i64 = store
            .connection
            .query_row("PRAGMA journal_size_limit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            limit, WAL_HEAL_THRESHOLD_BYTES,
            "without this the log never shrinks below its high-water mark"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn v4_structured_rows_read_identically_to_their_json_payload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("snapshots.sqlite");
        // One node of each shape: a measured one that lives in the columns,
        // and one whose payload is the only record of it.
        let mut mixed = graph("v4", 4096);
        mixed.nodes[1].size_known = false;
        let mut store = SqliteSnapshotStore::open(&path).unwrap();
        store.save(&mixed).unwrap();
        drop(store);

        // The `kind` column is the marker for which shape a row is in: set
        // means the columns carry the node, empty means the payload does.
        let connection = rusqlite::Connection::open(&path).unwrap();
        let rows: Vec<(String, Option<String>)> = connection
            .prepare("SELECT node_json, kind FROM nodes WHERE snapshot_id = ?1")
            .unwrap()
            .query_map([&mixed.snapshot.id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        let measured = rows
            .iter()
            .find(|(_, kind)| kind.is_some())
            .expect("a measured row carries the columns");
        assert!(measured.0.is_empty(), "and no copy of itself");
        let unmeasured = rows
            .iter()
            .find(|(_, kind)| kind.is_none())
            .expect("an unmeasured row keeps its payload");
        let parsed: DiskNode = serde_json::from_str(&unmeasured.0).unwrap();
        assert!(!parsed.size_known);

        // Both shapes read back as the nodes that were written.
        let store = SqliteSnapshotStore::open(&path).unwrap();
        let loaded = store.load(&mixed.snapshot.id).unwrap();
        assert_eq!(loaded.nodes.len(), mixed.nodes.len());
        for (loaded, written) in loaded.nodes.iter().zip(mixed.nodes.iter()) {
            assert_eq!(loaded, written, "the fast path must lose nothing");
        }
    }

    #[test]
    fn persists_across_connections() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("snapshots.sqlite");
        SqliteSnapshotStore::open(&db)
            .unwrap()
            .save(&graph("persistent", 123))
            .unwrap();
        assert_eq!(
            SqliteSnapshotStore::open(&db)
                .unwrap()
                .load("persistent")
                .unwrap()
                .nodes[1]
                .subtree_bytes,
            123
        );
    }

    /// Builds a genuine v1 database file (old schema, old user_version, one row)
    /// without going through the current code paths.
    fn write_v1_database(path: &std::path::Path) {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch(V1_SCHEMA).unwrap();
        connection
            .execute(
                "INSERT INTO snapshots (id, root_key, captured_at_unix_ms, snapshot_json)
                 VALUES ('v1-row', '\"/tmp/diskgraph-test\"', 5, '{\"id\":\"v1-row\",\"root\":{\"type\":\"value\",\"value\":\"\"}}')",
                [],
            )
            .unwrap();
        drop(connection);
    }

    #[test]
    fn v1_databases_migrate_in_place_keeping_existing_rows() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("graph.sqlite");
        write_v1_database(&db);
        let store = SqliteSnapshotStore::open(&db).unwrap();
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SUPPORTED_SCHEMA_VERSION);
        // The v1 row survives (its JSON here is a minimal stand-in; load of the
        // full snapshot is exercised through publish_revision tests below).
        let rows: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM snapshots", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn unknown_future_schema_versions_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("future.sqlite");
        let connection = Connection::open(&db).unwrap();
        connection
            .execute_batch("PRAGMA user_version = 99;")
            .unwrap();
        drop(connection);
        assert!(matches!(
            SqliteSnapshotStore::open(&db),
            Err(StoreError::UnsupportedSchema(99))
        ));
    }

    #[test]
    fn migration_backup_is_written_and_open_failure_keeps_v1_intact() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("graph.sqlite");
        write_v1_database(&db);
        let backup_dir = directory.path().join("backups");
        let (store, backup) = SqliteSnapshotStore::open_with_backup(&db, &backup_dir).unwrap();
        let backup = backup.expect("v1 open must produce a backup");
        assert!(backup.exists());
        let backup_version: i64 = Connection::open(&backup)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            backup_version, 1,
            "the backup must be the pre-migration bytes"
        );
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            SUPPORTED_SCHEMA_VERSION
        );
        // A fresh v2 open reports that no backup was needed.
        let (_, no_backup) = SqliteSnapshotStore::open_with_backup(&db, &backup_dir).unwrap();
        assert!(no_backup.is_none());
    }

    #[test]
    fn staging_is_invisible_until_published_and_latest_advances_atomically() {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let first = graph("one", 100);
        let second = graph("two", 150);

        // Stage a partial batch for a job that has not published yet.
        store.append_staging_nodes("job-1", &second.nodes).unwrap();
        assert_eq!(store.staging_node_count("job-1").unwrap(), 2);
        // Ordinary queries see nothing from staging...
        assert!(store.snapshot("two").is_err());
        assert_eq!(store.list_snapshots(None, 10, 0).unwrap().len(), 0);
        // ...and an aborted job leaves nothing behind.
        store.clear_staging("job-1").unwrap();
        assert_eq!(store.staging_node_count("job-1").unwrap(), 0);

        // Publish job-2 then job-3; latest follows each publication.
        store
            .publish_revision("job-2", &first, "rev-1", 1_000)
            .unwrap();
        assert_eq!(
            store
                .latest_revision_for_root(&first.snapshot.root)
                .unwrap(),
            Some("rev-1".into())
        );
        store
            .publish_revision("job-3", &second, "rev-2", 2_000)
            .unwrap();
        assert_eq!(
            store
                .latest_revision_for_root(&second.snapshot.root)
                .unwrap(),
            Some("rev-2".into())
        );
        assert_eq!(store.staging_node_count("job-3").unwrap(), 0);
        let loaded = store.load_revision("rev-2").unwrap();
        assert_eq!(loaded, second);
        assert_eq!(store.revision("rev-1").unwrap().snapshot_id, "one");
        assert!(store.revision("missing").is_err());
    }

    #[test]
    fn snapshot_removal_respects_pins_and_revision_dependencies() {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let pinned = graph("pinned", 10);
        let referenced = graph("referenced", 20);
        let plain = graph("plain", 30);
        store.save(&pinned).unwrap();
        store.save(&plain).unwrap();

        store.pin_snapshot("pinned", true).unwrap();
        assert!(matches!(
            store.remove_snapshot("pinned").unwrap_err(),
            StoreError::RetentionViolation(_)
        ));
        store.pin_snapshot("pinned", false).unwrap();
        store.remove_snapshot("pinned").unwrap();
        assert!(store.snapshot("pinned").is_err());

        // publish_revision creates the snapshot row itself.
        store
            .publish_revision("job-ref", &referenced, "rev-ref", 5)
            .unwrap();
        assert!(matches!(
            store.remove_snapshot("referenced").unwrap_err(),
            StoreError::RetentionViolation(_)
        ));
        store.remove_snapshot("plain").unwrap();
    }

    #[test]
    fn snapshots_list_pages_newest_first_and_can_filter_by_root() {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let mut other_root = graph("other-root", 50);
        other_root.snapshot.root = ResourceLocator::NativePath("/tmp/other".into());
        other_root.nodes[0].locator = ResourceLocator::NativePath("/tmp/other".into());
        store.save(&graph("old", 100)).unwrap();
        store.save(&graph("new", 150)).unwrap();
        store.save(&other_root).unwrap();

        let root = ResourceLocator::NativePath("/tmp/diskgraph-test".into());
        let same_root = store.list_snapshots(Some(&root), 10, 0).unwrap();
        assert_eq!(same_root.len(), 2);
        assert_eq!(same_root[0].id, "new");

        let page = store.list_snapshots(Some(&root), 1, 0).unwrap();
        assert_eq!(page.len(), 1);
        let second_page = store.list_snapshots(Some(&root), 1, 1).unwrap();
        assert_eq!(second_page[0].id, "old");

        assert_eq!(store.list_snapshots(None, 10, 0).unwrap().len(), 3);
    }

    #[test]
    fn collector_batches_store_validate_and_explain() {
        use diskgraph_core::{
            AssertionKind, CollectorRun, Entity, EntityKind, EvidenceRecord, Polarity, Relation,
            RelationEdge,
        };
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        let graph = graph("snap", 100);
        // publish_revision creates the snapshot row the batch attaches to.
        store.publish_revision("job", &graph, "rev-x", 1).unwrap();

        let run = CollectorRun {
            run_id: "run-1".into(),
            snapshot_id: "snap".into(),
            collector_id: "cargo-node-projects".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: 10,
            coverage_complete: true,
            errors: vec![],
        };
        let entities = vec![
            Entity {
                entity_id: "ent-res".into(),
                kind: EntityKind::Resource,
                identity: r#"{"node_id":2}"#.into(),
                display: "/tmp/diskgraph-test/cache".into(),
                source_run_id: "run-1".into(),
            },
            Entity {
                entity_id: "ent-project".into(),
                kind: EntityKind::Project,
                identity: r#"{"manifest":"Cargo.toml"}"#.into(),
                display: "diskgraph-test".into(),
                source_run_id: "run-1".into(),
            },
            Entity {
                entity_id: "ent-recipe".into(),
                kind: EntityKind::BuildRecipe,
                identity: r#"{"tool":"cargo"}"#.into(),
                display: "cargo build".into(),
                source_run_id: "run-1".into(),
            },
        ];
        let evidence = vec![EvidenceRecord {
            evidence_id: "ev-1".into(),
            run_id: "run-1".into(),
            basis: "target/ sits next to Cargo.toml in the same directory".into(),
            observed_at_unix_ms: 10,
            expires_at_unix_ms: None,
            confidence: 90,
            input_fingerprint: "fp-1".into(),
        }];
        let edges = vec![
            RelationEdge {
                edge_id: "edge-1".into(),
                source_entity_id: "ent-res".into(),
                relation: Relation::OwnedByProject,
                target_entity_id: "ent-project".into(),
                assertion_kind: AssertionKind::Observed,
                evidence_refs: vec![("ev-1".into(), Polarity::Supports)],
            },
            RelationEdge {
                edge_id: "edge-2".into(),
                source_entity_id: "ent-res".into(),
                relation: Relation::RebuildableBy,
                target_entity_id: "ent-recipe".into(),
                assertion_kind: AssertionKind::Derived,
                evidence_refs: vec![("ev-1".into(), Polarity::Supports)],
            },
        ];
        store
            .record_collector_batch("snap", &run, &entities, &evidence, &edges)
            .unwrap();
        store
            .bind_runs_to_revision("rev-x", &[("run-1", "active")])
            .unwrap();

        assert_eq!(
            store.edges_to("snap", "ent-project", None).unwrap().len(),
            1
        );
        assert_eq!(
            store
                .edges_from("snap", "ent-res", Some(Relation::RebuildableBy))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .edges_from("snap", "ent-res", Some(Relation::ProtectedBy))
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            store.entity("snap", "ent-project").unwrap().unwrap().kind,
            EntityKind::Project
        );
        let provenance = store
            .evidence_for_edges("snap", &["edge-1".into(), "edge-2".into()])
            .unwrap();
        assert_eq!(provenance.len(), 1);
        assert_eq!(provenance[0].evidence_id, "ev-1");
        // Cross-batch evidence references are refused.
        let dangling = vec![RelationEdge {
            edge_id: "edge-3".into(),
            source_entity_id: "ent-res".into(),
            relation: Relation::OwnedByProject,
            target_entity_id: "ent-project".into(),
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![("ev-missing".into(), Polarity::Supports)],
        }];
        assert!(
            store
                .record_collector_batch("snap", &run, &entities, &evidence, &dangling)
                .is_err()
        );
    }

    #[test]
    fn endpoint_violations_reject_the_whole_batch() {
        use diskgraph_core::{
            AssertionKind, CollectorRun, Entity, EntityKind, Relation, RelationEdge,
        };
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        store.save(&graph("snap", 100)).unwrap();
        let run = CollectorRun {
            run_id: "run-1".into(),
            snapshot_id: "snap".into(),
            collector_id: "bad".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: 1,
            coverage_complete: true,
            errors: vec![],
        };
        let entities = vec![
            Entity {
                entity_id: "ent-res".into(),
                kind: EntityKind::Resource,
                identity: "{}".into(),
                display: "r".into(),
                source_run_id: "run-1".into(),
            },
            Entity {
                entity_id: "ent-proc".into(),
                kind: EntityKind::Process,
                identity: "{}".into(),
                display: "p".into(),
                source_run_id: "run-1".into(),
            },
        ];
        let illegal = vec![RelationEdge {
            edge_id: "bad-edge".into(),
            source_entity_id: "ent-res".into(),
            relation: Relation::OwnedByProject,
            target_entity_id: "ent-proc".into(),
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![],
        }];
        assert!(
            store
                .record_collector_batch("snap", &run, &entities, &[], &illegal)
                .is_err()
        );
        // Nothing from the rejected batch may be visible.
        assert!(
            store
                .edges_from("snap", "ent-res", None)
                .unwrap()
                .is_empty()
        );
    }
}
