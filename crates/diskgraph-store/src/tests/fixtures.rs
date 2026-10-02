use crate::graph_migrations::V1_SCHEMA;
use crate::sqlite_snapshot_store::wal_path;
use diskgraph_core::{DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, ResourceLocator};
use diskgraph_core::{EvidenceRelation, NodeKind, ScanCoverage, ScanSettings};
use rusqlite::Connection;
use std::path::Path;

pub(crate) fn graph(id: &str, bytes: u64) -> DiskGraph {
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

/// The write-ahead log beside a database, as SQLite writes it.
pub(crate) fn wal_len(path: &Path) -> u64 {
    std::fs::metadata(wal_path(path))
        .map(|meta| meta.len())
        .unwrap_or(0)
}

/// Builds a genuine v1 database file (old schema, old user_version, one row)
/// without going through the current code paths.
pub(crate) fn write_v1_database(path: &std::path::Path) {
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
