//! 共享 Engine 的 scan_execution 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError, collect_projects};
use diskgraph_core::{
    Authorizer, BudgetDecision, BudgetUsage, BusinessError, DiskGraph, Permission, ScanBudget,
    ScanBudgetStop, ScanExclusions, ScanWindow,
};
use diskgraph_store::StoreError;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

impl Engine {
    /// 在当前 fence 下扫描、转换、暂存并原子发布 revision。
    /// 参数：job_id/owner/fence 确定代次，cancel 为该代次协作取消标志。
    /// 返回：成功或扫描/容量/授权/fence/存储失败；collector 失败不撤回已发布图。
    pub(super) fn execute_scan(
        &self,
        job_id: &str,
        owner: &str,
        fence: u64,
        cancel: &AtomicBool,
    ) -> Result<(), EngineError> {
        let job = self.control()?.job(job_id)?;
        if job.fencing_token != fence || job.owner != owner {
            return Err(StoreError::StaleOwner.into());
        }
        let scope = self.control()?.scope(&job.scope_id)?;
        if scope.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        if self.control()?.policy_state()?.is_some() {
            self.require(
                &self.policy_authorizer()?,
                &job.principal,
                &Permission::IndexWrite,
                &job.scope_id,
            )?;
        }
        let root = scope.root.to_native_path().map_err(|error| {
            EngineError::Store(StoreError::InvalidGraph(format!(
                "scope root is not addressable on this platform: {error}"
            )))
        })?;

        // 2.5: the walk is an observation over a window, recorded with the
        // options that produced it, so two snapshots are only comparable when
        // they were configured the same way.
        let started_at_unix_ms = now_ms();
        let mut last_heartbeat = started_at_unix_ms;
        let staging_id = format!("{job_id}:{}", job.fencing_token);
        let options = self.scan_options.clone();
        let handle =
            diskgraph_disktree_core::scan::ScanHandle::spawn(root.clone(), options.clone());
        let mut exceeded = false;
        let tree = loop {
            if now_ms().saturating_sub(last_heartbeat) >= 5000 {
                if self
                    .control()?
                    .heartbeat_fenced(job_id, owner, job.fencing_token)
                    .is_err()
                {
                    handle.cancel();
                    return Err(EngineError::Store(StoreError::StaleOwner));
                }
                last_heartbeat = now_ms();
            }
            let progress = handle.progress.snapshot();
            if let Ok(mut entries) = self.scan_progress.lock() {
                entries.insert((job_id.to_owned(), job.fencing_token), progress.clone());
            }
            if progress.files.saturating_add(progress.dirs)
                > self.max_nodes_per_scan.min(self.scan_budget.max_nodes)
                || now_ms().saturating_sub(started_at_unix_ms) > self.scan_budget.max_duration_ms
            {
                exceeded = true;
                handle.cancel();
            }
            if cancel.load(Ordering::SeqCst)
                || self.control()?.cancellation_requested(job_id, fence)?
                || self.scope(&job.scope_id)?.revoked
                || (self.control()?.policy_state()?.is_some()
                    && !matches!(
                        self.policy_authorizer()?.decide(
                            &job.principal,
                            &Permission::IndexWrite,
                            &job.scope_id
                        ),
                        diskgraph_core::Decision::Allowed
                    ))
            {
                cancel.store(true, Ordering::SeqCst);
                handle.cancel();
            }
            match handle.poll() {
                Some(result) => break result,
                None => thread::sleep(Duration::from_millis(20)),
            }
        };
        if exceeded {
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }
        let tree = tree.map_err(|error| {
            if cancel.load(Ordering::SeqCst) {
                EngineError::Business(BusinessError::Conflict)
            } else {
                EngineError::Io(error)
            }
        })?;
        let scanned = diskgraph_disktree::convert_tree(&root, &tree, scan_settings(&options))
            .map_err(|error| {
                if error.kind() == io::ErrorKind::Unsupported {
                    EngineError::Business(BusinessError::Unsupported)
                } else {
                    EngineError::Io(error)
                }
            })?;
        drop(tree);
        let window = ScanWindow {
            started_at_unix_ms,
            finished_at_unix_ms: now_ms(),
            options_fingerprint: options_fingerprint(&options),
            scanner_version: env!("CARGO_PKG_VERSION").to_owned(),
        };

        // 2.9: charge every observed node against the walk budget, so a scan
        // that exceeds a limit stops for a named reason instead of quietly
        // returning less than it found.
        let mut usage = BudgetUsage {
            elapsed_ms: window.duration_ms(),
            ..BudgetUsage::default()
        };
        let mut exclusions = ScanExclusions::default();
        let mut budget_stop: Option<ScanBudgetStop> = None;
        let total_bytes: u64 = scanned.nodes.iter().map(|node| node.v1.subtree_bytes).sum();

        // RT-04: the configured node ceiling is a hard refusal.
        if scanned.nodes.len() as u64 > self.max_nodes_per_scan {
            let mut graph = self.graph()?;
            let _ = graph.clear_staging(&staging_id);
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }

        // 2.9: charge the walk against its budget. A reached limit stops the
        // walk for a named reason instead of returning less data silently, and
        // a cancellation observed here is reported, never swallowed. The byte
        // charge is the node's OWN bytes, never its subtree aggregate: every
        // ancestor would otherwise bill the same file again, so a deep tree
        // would multiply its real size by its depth and stop on phantom bytes.
        for node in &scanned.nodes {
            match self.scan_budget.charge_node(
                &mut usage,
                (serde_json::to_vec(&node.v1)
                    .map_err(StoreError::from)?
                    .len()
                    + node.v1.name.to_lowercase().len()
                    + (match &node.v1.locator {
                        diskgraph_core::ResourceLocator::NativePath(path)
                        | diskgraph_core::ResourceLocator::DocumentUri(path) => path,
                    })
                    .to_lowercase()
                    .len()) as u64,
            ) {
                BudgetDecision::Continue => {}
                BudgetDecision::Stop(stop) => {
                    budget_stop = Some(stop);
                    break;
                }
            }
            if let Some(decision) = ScanBudget::observe_cancel(cancel.load(Ordering::SeqCst))
                && decision.is_stop()
            {
                budget_stop = Some(ScanBudgetStop::Cancelled);
                break;
            }
        }
        if let Some(stop) = budget_stop {
            // Record why the walk ended before refusing, so the job log can
            // explain itself. The named reason goes to the operator's stderr
            // too: a failed job must be diagnosable without a debugger.
            exclusions
                .error_summary
                .push(format!("scan stopped: {stop:?}"));
            eprintln!(
                "diskgraph: scan stopped: {stop:?} after {} nodes / {} charged bytes / {} ms",
                usage.nodes, usage.staged_bytes, usage.elapsed_ms
            );
            let _ = total_bytes;
            let mut graph = self.graph()?;
            let _ = graph.clear_staging(&staging_id);
            return Err(EngineError::Business(match stop {
                ScanBudgetStop::Cancelled => BusinessError::Conflict,
                _ => BusinessError::BudgetExceeded,
            }));
        }

        // Stage in bounded batches, then publish snapshot + revision + latest
        // pointer in one transaction (ST-01).
        let mut graph = self.graph()?;
        for batch in scanned
            .nodes
            .chunks(self.scan_budget.write_batch_nodes.max(1) as usize)
        {
            if !self.accepts_new_work() {
                graph.clear_staging(&staging_id)?;
                return Err(EngineError::Business(BusinessError::ResourceExhausted));
            }
            self.control()?
                .heartbeat_fenced(job_id, owner, job.fencing_token)?;
            self.control()?
                .with_job_fence(job_id, owner, job.fencing_token, || {
                    graph.append_staging_iter(&staging_id, batch.iter().map(|node| &node.v1))
                })?;
        }
        let v1_nodes: Vec<diskgraph_core::DiskNode> =
            scanned.nodes.into_iter().map(|node| node.v1).collect();
        let revision_id = format!("rev-{}-{}", job_id, job.fencing_token);
        let published_at = now_ms();
        let observed_graph = DiskGraph {
            snapshot: scanned.snapshot,
            nodes: v1_nodes,
            evidence: scanned.evidence,
        };
        if !self.accepts_new_work() {
            graph.clear_staging(&staging_id)?;
            return Err(EngineError::Business(BusinessError::ResourceExhausted));
        }
        // 撤权和发布共用控制库锁，防止检查通过后撤权仍发布。
        let mut control = self.control()?;
        if control.scope(&job.scope_id)?.revoked || cancel.load(Ordering::SeqCst) {
            graph.clear_staging(&staging_id)?;
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        if control.policy_state()?.is_some() {
            Self::require_with_control(
                &control,
                &control.authorizer()?,
                &job.principal,
                &Permission::IndexWrite,
                &job.scope_id,
            )?;
        }
        let server_id = control.ensure_server()?;
        control.heartbeat_fenced(job_id, owner, job.fencing_token)?;
        let result = control.with_job_fence(job_id, owner, job.fencing_token, || {
            graph.publish_revision_owned(
                &staging_id,
                &observed_graph,
                &revision_id,
                published_at,
                Some((server_id.as_str(), job.scope_id.as_str())),
            )
        });
        drop(control);
        if let Err(error) = result {
            let _ = graph.clear_staging(&staging_id);
            return Err(error.into());
        }

        // Deterministic collectors run against the just-published snapshot and
        // bind their run to the revision as the active evidence batch (EV-05).
        // A collector failure fails the job but never un-publishes the scan.
        drop(graph);
        let batch = collect_projects(&observed_graph);
        let mut graph = self.graph()?;
        if !batch.edges.is_empty() || !batch.entities.is_empty() {
            self.control()?
                .with_job_fence(job_id, owner, job.fencing_token, || {
                    graph.record_collector_batch(
                        &observed_graph.snapshot.id,
                        &batch.run,
                        &batch.entities,
                        &batch.evidence,
                        &batch.edges,
                    )?;
                    graph.bind_runs_to_revision(
                        &revision_id,
                        &[(batch.run.run_id.as_str(), "active")],
                    )
                })?;
        }
        Ok(())
    }
}

/// A stable fingerprint of the scan options, so two snapshots can be compared
/// only when they were produced the same way (FS-01).
fn options_fingerprint(options: &diskgraph_disktree_core::scan::ScanOptions) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    options.apparent_size.hash(&mut hasher);
    options.follow_links.hash(&mut hasher);
    options.include_hidden.hash(&mut hasher);
    options.one_filesystem.hash(&mut hasher);
    options.max_depth.hash(&mut hasher);
    options.dedup_hardlinks.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
fn scan_settings(
    options: &diskgraph_disktree_core::scan::ScanOptions,
) -> diskgraph_core::ScanSettings {
    diskgraph_core::ScanSettings {
        apparent_size: options.apparent_size,
        follow_links: options.follow_links,
        include_hidden: options.include_hidden,
        one_filesystem: options.one_filesystem,
        max_depth: options.max_depth,
        dedup_hardlinks: options.dedup_hardlinks,
    }
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis()
        .try_into()
        .expect("timestamp beyond u64")
}
