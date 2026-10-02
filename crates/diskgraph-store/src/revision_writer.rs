//! staging 到正式 revision 的原子发布。

use crate::directory_aggregates;

use crate::graph_validation::validate_graph;
use crate::node_codec::{as_i64, measured_kind, payload_for};
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{DiskGraph, ResourceLocator};
use rusqlite::params;
use serde_json::to_string;

impl SqliteSnapshotStore {
    /// Publishes one scan atomically: snapshot rows, the graph revision, the
    /// per-root latest pointer, and staging cleanup share one transaction
    /// (spec ST-01). A failure leaves the previous latest untouched.
    /// 在同一图库事务校验并发布 revision、聚合与 latest。
    /// 参数：job_id：本 fencing 代次的暂存命名空间；graph：完整观测图；revision_id：已发布 revision ID；published_at_unix_ms：Unix 毫秒发布时间。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn publish_revision(
        &mut self,
        job_id: &str,
        graph: &DiskGraph,
        revision_id: &str,
        published_at_unix_ms: u64,
    ) -> Result<()> {
        self.publish_revision_owned(job_id, graph, revision_id, published_at_unix_ms, None)
    }

    /// 在发布事务中绑定 revision 的实际 server/scope；None 仅用于可信内部兼容接口。
    /// 在同一图库事务校验并发布 revision、聚合与 latest。
    /// 参数：job_id：本 fencing 代次的暂存命名空间；graph：完整观测图；revision_id：已发布 revision ID；published_at_unix_ms：Unix 毫秒发布时间；ownership：可选已授权 server/scope 归属。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn publish_revision_owned(
        &mut self,
        job_id: &str,
        graph: &DiskGraph,
        revision_id: &str,
        published_at_unix_ms: u64,
        ownership: Option<(&str, &str)>,
    ) -> Result<()> {
        validate_graph(graph)?;
        let root_key = to_string(&graph.snapshot.root)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO snapshots (id, root_key, captured_at_unix_ms, snapshot_json, pinned, count_schema)
             VALUES (?1, ?2, ?3, ?4, 0, 9)",
            params![
                graph.snapshot.id,
                root_key,
                as_i64(graph.snapshot.captured_at_unix_ms)?,
                to_string(&graph.snapshot)?,
            ],
        )?;
        {
            if ownership.is_some() {
                let count: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM scan_staging WHERE job_id = ?1",
                    [job_id],
                    |row| row.get(0),
                )?;
                if count as usize != graph.nodes.len() {
                    return Err(StoreError::InvalidGraph(
                        "staging does not match the completed scan".into(),
                    ));
                }
                transaction.execute("INSERT INTO nodes (snapshot_id, id, parent_id, locator_key, name, subtree_bytes, node_json, kind, direct_bytes, files, directories, modified_unix_seconds, file_volume_id, file_id, category_hint, reclaim_hint, read_error) SELECT ?2, json_extract(node_json, '$.id'), json_extract(node_json, '$.parent_id'), json_extract(node_json, '$.locator'), json_extract(node_json, '$.name'), json_extract(node_json, '$.subtree_bytes'), CASE WHEN json_extract(node_json, '$.size_known') = 1 THEN '' ELSE node_json END, CASE WHEN json_extract(node_json, '$.size_known') = 1 THEN json_extract(node_json, '$.kind') ELSE NULL END, json_extract(node_json, '$.direct_bytes'), json_extract(node_json, '$.files'), json_extract(node_json, '$.directories'), json_extract(node_json, '$.modified_unix_seconds'), json_extract(node_json, '$.file_identity.volume_id'), json_extract(node_json, '$.file_identity.file_id'), json_extract(node_json, '$.category_hint'), json_extract(node_json, '$.reclaim_hint'), json_extract(node_json, '$.read_error') FROM scan_staging WHERE job_id = ?1", params![job_id, graph.snapshot.id])?;
                transaction.execute("INSERT INTO node_search SELECT ?2, json_extract(s.node_json, '$.id'), f.name_fold, f.path_fold FROM scan_staging s JOIN scan_staging_search f ON f.job_id = s.job_id AND f.node_seq = s.node_seq WHERE s.job_id = ?1", params![job_id, graph.snapshot.id])?;
            } else {
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
                        ResourceLocator::NativePath(path) | ResourceLocator::DocumentUri(path) => {
                            path
                        }
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
        if let Some((server_id, scope_id)) = ownership {
            transaction.execute(
                "INSERT INTO revision_ownership VALUES (?1, ?2, ?3)",
                params![revision_id, server_id, scope_id],
            )?;
        }
        transaction.execute(
            "INSERT INTO latest_revision (root_key, revision_id) VALUES (?1, ?2)
             ON CONFLICT(root_key) DO UPDATE SET revision_id = ?2",
            params![root_key, revision_id],
        )?;
        transaction.execute("DELETE FROM scan_staging WHERE job_id = ?1", [job_id])?;
        transaction.execute(
            "DELETE FROM scan_staging_search WHERE job_id = ?1",
            [job_id],
        )?;
        directory_aggregates::rebuild(&transaction, Some(&graph.snapshot.id))?;
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
}
