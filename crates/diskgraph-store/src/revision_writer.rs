//! staging 到正式 revision 的原子发布。

use crate::directory_aggregates;

use crate::graph_validation::validate_graph_display_aliases;
use crate::node_codec::{as_i64, measured_kind, payload_for};
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{CollectorBatch, DiskGraph, ResourceLocator};
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
        self.publish_revision_owned_with_batch(
            job_id,
            graph,
            revision_id,
            published_at_unix_ms,
            ownership,
            None,
        )
    }

    /// 在扫描发布事务内同时写入可选采集批次，并将其绑定为 active。
    /// 参数：暂存任务、文件图、revision、时间、实际归属及可选项目采集批次。
    /// 返回：文件图、采集数据与最新指针一起提交，任一失败整体回滚。
    pub fn publish_revision_owned_with_batch(
        &mut self,
        job_id: &str,
        graph: &DiskGraph,
        revision_id: &str,
        published_at_unix_ms: u64,
        ownership: Option<(&str, &str)>,
        batch: Option<&CollectorBatch>,
    ) -> Result<()> {
        self.publish_revision_owned_with_batch_checked(
            job_id,
            graph,
            (revision_id, published_at_unix_ms),
            ownership,
            batch,
            || Ok(()),
        )?;
        // 可信兼容入口保留同步维护；持有任务 fence 的调用方使用 checked 入口单独结算。
        let _ = self.checkpoint_after_publication();
        Ok(())
    }

    /// 在真实图库事务提交前复核原认证、取消及当前租约时钟。
    /// 参数：暂存代次、图、(revision ID,发布时间)、实际归属、可选批次及纯检查回调。
    /// 返回：全部提交或回滚；回调须只读既有不可变上下文，禁止重入持有的控制库。
    /// 旧接口保持可信兼容包装；检查后的 commit 仍不构成跨库或硬墙钟原子承诺。
    /// 此入口不执行提交后的显式 checkpoint，任务调用方须先结算终态再维护 WAL。
    pub fn publish_revision_owned_with_batch_checked(
        &mut self,
        job_id: &str,
        graph: &DiskGraph,
        revision: (&str, u64),
        ownership: Option<(&str, &str)>,
        batch: Option<&CollectorBatch>,
        check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        self.publish_revision_with_receipt_inner(
            job_id, graph, revision, ownership, batch, check, None,
        )
    }

    /// 参数：本代 staging、完整观测、原任务回执、采集批次及原末检；返回：结果与回执同事务提交或全部回滚。
    /// 原请求和 fence 仍由调用者在控制事务内核验；回执本身不授予执行权限。
    pub fn publish_scan_revision_checked(
        &mut self,
        staging_id: &str,
        graph: &DiskGraph,
        receipt: &crate::ScanPublicationReceipt,
        batch: Option<&CollectorBatch>,
        check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        receipt.validate()?;
        if staging_id != format!("{}:{}", receipt.job_id, receipt.publishing_fence)
            || graph.snapshot.id != receipt.snapshot_id
        {
            return Err(StoreError::InvalidGraph(
                "scan receipt result mismatch".into(),
            ));
        }
        self.publish_revision_with_receipt_inner(
            staging_id,
            graph,
            (&receipt.revision_id, receipt.published_at_unix_ms),
            Some((receipt.server_id.as_str(), receipt.scope_id.as_str())),
            batch,
            check,
            Some(receipt),
        )
    }

    // 保留旧可信入口；原子扫描发布额外携带同事务唯一回执，不复制完整发布实现。
    #[allow(clippy::too_many_arguments)]
    fn publish_revision_with_receipt_inner(
        &mut self,
        job_id: &str,
        graph: &DiskGraph,
        revision: (&str, u64),
        ownership: Option<(&str, &str)>,
        batch: Option<&CollectorBatch>,
        mut check: impl FnMut() -> Result<()>,
        receipt: Option<&crate::ScanPublicationReceipt>,
    ) -> Result<()> {
        let (revision_id, published_at_unix_ms) = revision;
        let started = std::time::Instant::now();
        let diagnostic = std::env::var_os("DISKGRAPH_SCAN_DIAGNOSTICS").as_deref()
            == Some(std::ffi::OsStr::new("1"));
        // 单调计时仅用于分离实际 SQL/commit 与提交后维护，禁止输出资源标识。
        let trace = |phase: &str| {
            if diagnostic {
                eprintln!(
                    "diskgraph: publish_phase={phase} elapsed_ms={}",
                    started.elapsed().as_millis()
                );
            }
        };
        trace("prepare_begin");
        check()?;
        let has_display_aliases = validate_graph_display_aliases(graph, ownership.is_some())?;
        let root_key = to_string(&graph.snapshot.root)?;
        let transaction = self.connection.transaction()?;
        check()?;
        transaction.execute(
            "INSERT INTO snapshots (id, root_key, captured_at_unix_ms, snapshot_json, pinned, count_schema, scan_receipt_writer)
             VALUES (?1, ?2, ?3, ?4, 0, 9, 16)",
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
                if has_display_aliases {
                    crate::staging_locator_validation::validate_aliases(
                        &transaction,
                        job_id,
                        graph,
                    )?;
                }
                transaction.execute("INSERT INTO nodes (snapshot_id, id, parent_id, locator_key, name, subtree_bytes, node_json, kind, direct_bytes, files, directories, modified_unix_seconds, file_volume_id, file_id, category_hint, reclaim_hint, read_error, native_locator_kind, native_locator_encoding, native_locator_raw, self_modified_unix_seconds, native_observation_format, native_observation_raw, native_observation_gap) SELECT ?2, json_extract(node_json, '$.id'), json_extract(node_json, '$.parent_id'), json_extract(node_json, '$.locator'), json_extract(node_json, '$.name'), json_extract(node_json, '$.subtree_bytes'), CASE WHEN json_extract(node_json, '$.size_known') = 1 THEN '' ELSE node_json END, CASE WHEN json_extract(node_json, '$.size_known') = 1 THEN json_extract(node_json, '$.kind') ELSE NULL END, json_extract(node_json, '$.direct_bytes'), json_extract(node_json, '$.files'), json_extract(node_json, '$.directories'), json_extract(node_json, '$.modified_unix_seconds'), json_extract(node_json, '$.file_identity.volume_id'), json_extract(node_json, '$.file_identity.file_id'), json_extract(node_json, '$.category_hint'), json_extract(node_json, '$.reclaim_hint'), json_extract(node_json, '$.read_error'), native_locator_kind, native_locator_encoding, native_locator_raw, self_modified_unix_seconds, native_observation_format, native_observation_raw, native_observation_gap FROM scan_staging WHERE job_id = ?1", params![job_id, graph.snapshot.id])?;
                transaction.execute("INSERT INTO node_search SELECT ?2, json_extract(s.node_json, '$.id'), f.name_fold, f.path_fold FROM scan_staging s JOIN scan_staging_search f ON f.job_id = s.job_id AND f.node_seq = s.node_seq WHERE s.job_id = ?1", params![job_id, graph.snapshot.id])?;
                check()?;
            } else {
                let mut statement = transaction.prepare(
                "INSERT INTO nodes (snapshot_id, id, parent_id, locator_key, name, subtree_bytes,
                 node_json, kind, direct_bytes, files, directories, modified_unix_seconds,
                 file_volume_id, file_id, category_hint, reclaim_hint, read_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )?;
                for node in &graph.nodes {
                    check()?;
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
                check()?;
                evidence.execute(params![
                    graph.snapshot.id,
                    as_i64(edge.node_id)?,
                    to_string(edge)?,
                ])?;
            }
        }
        trace("nodes_and_evidence_complete");
        crate::unix_observation_staging::publish(&transaction, job_id, &graph.snapshot.id)?;
        check()?;
        transaction.execute(
            "INSERT INTO graph_revisions (revision_id, snapshot_id, published_at_unix_ms, writer_generation, locator_writer_generation, native_observation_writer_generation)
             VALUES (?1, ?2, ?3, 10, 11, 12)",
            params![
                revision_id,
                graph.snapshot.id,
                as_i64(published_at_unix_ms)?
            ],
        )?;
        if let Some(batch) = batch {
            check()?;
            crate::collector_batch_writer::write_batch(
                &transaction,
                &graph.snapshot.id,
                &batch.run,
                &batch.entities,
                &batch.evidence,
                &batch.edges,
            )?;
            crate::collector_batch_writer::bind_runs(
                &transaction,
                revision_id,
                &graph.snapshot.id,
                &[(&batch.run.run_id, "active")],
            )?;
        }
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
        trace("aggregates_begin");
        directory_aggregates::rebuild(&transaction, Some(&graph.snapshot.id))?;
        trace("aggregates_complete");
        crate::collector_protocol::seal(&transaction, revision_id)?;
        if let Some(receipt) = receipt {
            crate::scan_receipt_query::write(&transaction, receipt)?;
        }
        trace("seal_complete");
        // 必须位于最后一次 SQL 和实际 commit 之间；失败回滚 latest、归属、节点及 staging 清理。
        check().inspect_err(|_| trace("terminal_check_failed"))?;
        trace("commit_begin");
        transaction
            .commit()
            .inspect_err(|_| trace("commit_failed"))?;
        trace("commit_complete");
        Ok(())
    }

    /// 提交后尝试折叠和截断 WAL，不承担发布或任务状态转换。
    /// 参数：self 为原图库写连接；任务调用方必须已释放控制库 fence 并结算终态。
    /// 返回：SQLite 维护调用结果；读取事务可阻止截断，错误不回滚已提交 revision。
    pub fn checkpoint_after_publication(&self) -> Result<()> {
        let started = std::time::Instant::now();
        let diagnostics =
            std::env::var_os("DISKGRAPH_SCAN_DIAGNOSTICS").is_some_and(|value| value == "1");
        if diagnostics {
            eprintln!("diskgraph: publish_phase=checkpoint_begin");
        }
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        if diagnostics {
            eprintln!(
                "diskgraph: publish_phase=checkpoint_complete elapsed_ms={}",
                started.elapsed().as_millis()
            );
        }
        Ok(())
    }
}
