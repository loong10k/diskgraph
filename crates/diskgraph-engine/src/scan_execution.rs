//! 共享 Engine 的 scan_execution 职责；原调用与持锁顺序保持。

use crate::scan_node_locator::qualify_scan_locator;
use crate::scan_observation_guard::ScanObservationGuard;
use crate::{Engine, EngineError, collect_projects};
use diskgraph_core::{
    Authorizer, BudgetDecision, BudgetUsage, BusinessError, DiskGraph, Permission, ScanBudgetStop,
    ScanWindow, WindowsFileObservation, WindowsObservationGap,
};
use diskgraph_store::StoreError;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

impl Engine {
    /// 在当前 fence 下扫描、转换、暂存并原子发布 revision。
    /// 参数：job_id/owner/fence 确定代次，cancel 为该代次协作取消标志。
    /// 返回：成功或扫描/容量/授权/fence/存储失败；采集发布失败整批回滚，既有版本保持不变。
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
        let scan_started = Instant::now();
        let mut last_heartbeat = started_at_unix_ms;
        let staging_id = format!("{job_id}:{}", job.fencing_token);
        let options = self.scan_options.clone();
        let observation_guard = ScanObservationGuard::new(self, &job, cancel, scan_started);
        observation_guard.check_now()?;
        #[cfg(windows)]
        let _hydration_guard = diskgraph_disktree::HydrationGuard::enter()
            .map_err(|_| EngineError::Business(BusinessError::Unsupported))?;
        #[cfg(windows)]
        let native_root =
            crate::windows_native_scan_root::WindowsNativeScanRoot::open(&root, &|| {
                observation_guard.check()
            })?;
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
                || scan_started.elapsed() > Duration::from_millis(self.scan_budget.max_duration_ms)
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
        if scanned.nodes.len() as u64 > self.max_nodes_per_scan.min(self.scan_budget.max_nodes) {
            return Err(BusinessError::BudgetExceeded.into());
        }

        // 采样和编码发生于数据库写锁外，额外对象只保留当前配置批次。
        for batch in scanned
            .nodes
            .chunks(self.scan_budget.write_batch_nodes.max(1) as usize)
        {
            observation_guard.check_now()?;
            if !self.accepts_new_work() {
                return Err(BusinessError::ResourceExhausted.into());
            }
            let mut locators = Vec::with_capacity(batch.len());
            let mut observations: Vec<(
                Option<WindowsFileObservation>,
                Option<WindowsObservationGap>,
            )> = Vec::with_capacity(batch.len());
            for node in batch {
                observation_guard.check()?;
                let locator = qualify_scan_locator(node)?;
                #[cfg(windows)]
                let observed = native_root.observe(
                    &locator
                        .to_native_path()
                        .map_err(|_| BusinessError::Unsupported)?,
                    &node.v1,
                    node.self_modified,
                    &scanned.snapshot.settings,
                    &|| observation_guard.check(),
                )?;
                #[cfg(not(windows))]
                let observed = (None, Some(WindowsObservationGap::Unsupported));
                #[cfg(test)]
                crate::scan_observation_tests::after_observe(&job.job_id);
                observation_guard.check()?;
                usage.elapsed_ms =
                    u64::try_from(scan_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let cost = diskgraph_store::staging_observed_node_encoded_cost(
                    &node.v1,
                    Some(&locator),
                    node.self_modified,
                    observed.0.as_ref(),
                    observed.1,
                )?;
                if let BudgetDecision::Stop(stop) = self.scan_budget.charge_node(&mut usage, cost) {
                    eprintln!(
                        "diskgraph: scan stopped: {stop:?} after {} nodes / {} charged bytes / {} ms",
                        usage.nodes, usage.staged_bytes, usage.elapsed_ms
                    );
                    return Err(match stop {
                        ScanBudgetStop::Cancelled => BusinessError::Conflict,
                        _ => BusinessError::BudgetExceeded,
                    }
                    .into());
                }
                locators.push(locator);
                observations.push(observed);
            }
            observation_guard.check_now()?;
            // 原生调用已结束；遵循既有 graph→control 写锁顺序，并在原子 fence 下暂存。
            #[cfg(test)]
            crate::scan_observation_tests::before_stage_lock(&job.job_id);
            let mut graph = self.graph()?;
            self.control()?
                .with_job_fence(job_id, owner, job.fencing_token, || {
                    // 已等待图锁和控制事务，写入前只检查本代次原始时钟与取消，避免重入控制锁。
                    if cancel.load(Ordering::SeqCst) {
                        return Err(StoreError::Conflict("scan cancelled before staging".into()));
                    }
                    if scan_started.elapsed()
                        > Duration::from_millis(self.scan_budget.max_duration_ms)
                    {
                        return Err(StoreError::BudgetExceeded);
                    }
                    graph.append_staging_observed_iter(
                        &staging_id,
                        batch.iter().zip(&locators).zip(&observations).map(
                            |((node, locator), (observed, gap))| {
                                (
                                    &node.v1,
                                    locator,
                                    node.self_modified,
                                    observed.as_ref(),
                                    *gap,
                                )
                            },
                        ),
                    )
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
        // 项目证据在公开 revision 前准备；计算期间不占用图库写锁。
        let project = collect_projects(&observed_graph);
        let collector = diskgraph_core::CollectorBatch {
            run: project.run,
            entities: project.entities,
            evidence: project.evidence,
            edges: project.edges,
        };
        observation_guard.check_now()?;
        #[cfg(windows)]
        native_root.validate_root(&|| observation_guard.check())?;
        let mut graph = self.graph()?;
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
            if cancel.load(Ordering::SeqCst) {
                return Err(StoreError::Conflict(
                    "scan cancelled before publication".into(),
                ));
            }
            let elapsed = scan_started.elapsed();
            #[cfg(test)]
            let elapsed = crate::scan_publication_tests::publication_elapsed(job_id, elapsed);
            // 包括转换、暂存、项目采集和锁等待；提交前再次协作检查整次扫描期限。
            if elapsed > Duration::from_millis(self.scan_budget.max_duration_ms) {
                return Err(StoreError::BudgetExceeded);
            }
            graph.publish_revision_owned_with_batch(
                &staging_id,
                &observed_graph,
                &revision_id,
                published_at,
                Some((server_id.as_str(), job.scope_id.as_str())),
                Some(&collector),
            )
        });
        drop(control);
        if let Err(error) = result {
            let _ = graph.clear_staging(&staging_id);
            return Err(error.into());
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
