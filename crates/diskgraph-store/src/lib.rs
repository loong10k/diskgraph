//! Immutable, per-node SQLite snapshots. No mutation of scanned resources.

use std::collections::HashSet;
use std::path::Path;

use diskgraph_core::{DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, ResourceLocator};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{from_str, to_string};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),
    #[error("invalid graph: {0}")]
    InvalidGraph(String),
    #[error("value is too large for SQLite INTEGER")]
    IntegerOverflow,
    #[error("unsupported SQLite schema version: {0}")]
    UnsupportedSchema(i64),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// A store owns only observed metadata. It never opens or removes scanned paths.
pub struct SqliteSnapshotStore {
    connection: Connection,
}

impl SqliteSnapshotStore {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self> {
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != 0 && version != 1 {
            return Err(StoreError::UnsupportedSchema(version));
        }
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
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
             CREATE INDEX IF NOT EXISTS nodes_by_locator
                 ON nodes (snapshot_id, locator_key);
             CREATE TABLE IF NOT EXISTS evidence (
                 snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
                 node_id INTEGER NOT NULL,
                 evidence_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS evidence_by_node
                 ON evidence (snapshot_id, node_id);
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
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
                "INSERT INTO nodes (snapshot_id, id, parent_id, locator_key, name, subtree_bytes, node_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for node in &graph.nodes {
                statement.execute(params![
                    graph.snapshot.id,
                    as_i64(node.id)?,
                    node.parent_id.map(as_i64).transpose()?,
                    to_string(&node.locator)?,
                    node.name,
                    as_i64(node.subtree_bytes)?,
                    to_string(node)?,
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
        let nodes = self.select_json(
            "SELECT node_json FROM nodes WHERE snapshot_id = ?1 ORDER BY id",
            id,
        )?;
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

    pub fn node(&self, snapshot_id: &str, node_id: u64) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT node_json FROM nodes WHERE snapshot_id = ?1 AND id = ?2",
                params![snapshot_id, as_i64(node_id)?],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| from_str(&json).map_err(StoreError::from))
            .transpose()
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
            "SELECT node_json FROM nodes
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
            |row| row.get::<_, String>(0),
        )?;
        rows.map(|row| Ok(from_str(&row?)?)).collect()
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

    pub fn node_by_locator(
        &self,
        snapshot_id: &str,
        locator: &ResourceLocator,
    ) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT node_json FROM nodes
                 WHERE snapshot_id = ?1 AND locator_key = ?2 LIMIT 1",
                params![snapshot_id, to_string(locator)?],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    fn select_json<T: serde::de::DeserializeOwned>(
        &self,
        sql: &str,
        snapshot_id: &str,
    ) -> Result<Vec<T>> {
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map([snapshot_id], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(from_str(&row?)?)).collect()
    }
}

fn as_i64(value: u64) -> Result<i64> {
    value.try_into().map_err(|_| StoreError::IntegerOverflow)
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
}
