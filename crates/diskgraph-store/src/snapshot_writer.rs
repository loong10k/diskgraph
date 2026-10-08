//! 不可变快照的完整写入事务。

use crate::directory_aggregates;

use crate::graph_validation::validate_graph;
use crate::node_codec::{as_i64, measured_kind, payload_for};
use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::{DiskGraph, ResourceLocator};
use rusqlite::params;
use serde_json::to_string;

impl SqliteSnapshotStore {
    /// Inserts a complete immutable observation atomically; duplicate IDs fail.
    /// 验证完整观测后在同一事务写入不可变快照。
    /// 参数：graph：完整观测图。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn save(&mut self, graph: &DiskGraph) -> Result<()> {
        validate_graph(graph)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO snapshots (id, root_key, captured_at_unix_ms, snapshot_json, count_schema, scan_receipt_writer)
             VALUES (?1, ?2, ?3, ?4, 9, 16)",
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
                let path = match &node.locator {
                    ResourceLocator::NativePath(path) | ResourceLocator::DocumentUri(path) => path,
                };
                transaction.execute(
                    "INSERT INTO node_search VALUES (?1, ?2, ?3, ?4)",
                    params![
                        graph.snapshot.id,
                        as_i64(node.id)?,
                        node.name.to_lowercase(),
                        path.to_lowercase()
                    ],
                )?;
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
        directory_aggregates::rebuild(&transaction, Some(&graph.snapshot.id))?;
        transaction.commit()?;
        Ok(())
    }
}
