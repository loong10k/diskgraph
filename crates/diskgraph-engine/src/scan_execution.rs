//! 共享 Engine 的 scan_execution 职责；原调用与持锁顺序保持。

use crate::job_execution_stop_reason::JobExecutionStopReason;
use crate::scan_node_locator::qualify_scan_locator;
use crate::scan_observation_guard::ScanObservationGuard;
use crate::{Engine, EngineError, collect_projects};
#[cfg(not(windows))]
use diskgraph_core::WindowsObservationGap;
use diskgraph_core::{
    Authorizer, BudgetDecision, BudgetUsage, BusinessError, DiskGraph, Permission, ScanBudgetStop,
    ScanWindow,
};
use diskgraph_store::{PreparedStagingNode, StoreError};
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

impl Engine {
    /// 在当前 fence 下扫描、转换、暂存并原子发布 revision。
    /// 参数：job_id/owner/fence 确定代次，cancel 为协作取消，scan_started 为原起点，stop_reason 保存同代 keeper 原失败。
    /// 返回：成功或扫描/容量/授权/fence/存储失败；采集发布失败整批回滚，既有版本保持不变。
    pub(super) fn execute_scan(
        &self,
        job_id: &str,
        owner: &str,
        fence: u64,
        cancel: &AtomicBool,
        scan_started: Instant,
        stop_reason: &JobExecutionStopReason,
    ) -> Result<(), EngineError> {
        // 进入执行前先扣除认领后的准备/锁等待；不能到扫描器启动时重新计时。
        if scan_started.elapsed() > Duration::from_millis(self.scan_budget.max_duration_ms) {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let job = self.control()?.job(job_id)?;
        if job.fencing_token != fence || job.owner != owner {
            return Err(StoreError::StaleOwner.into());
        }
        let authority = self.control()?.job_request_authority(job_id)?;
        if let Some(authority) = &authority {
            authority.validate_at(crate::job_authorization::unix_seconds()?)?;
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
        let observation_guard =
            ScanObservationGuard::new(self, &job, authority.as_ref(), cancel, scan_started)
                .with_stop_reason(stop_reason);
        observation_guard.check_now()?;
        #[cfg(windows)]
        let _hydration_guard = diskgraph_disktree::HydrationGuard::enter()
            .map_err(|_| EngineError::Business(BusinessError::Unsupported))?;
        #[cfg(windows)]
        let native_root =
            crate::windows_native_scan_root::WindowsNativeScanRoot::open(&root, &|| {
                observation_guard.check()
            })?;
        #[cfg(target_os = "linux")]
        let unix_root =
            crate::native_process::LinuxScanRoot::open(&root, &|| observation_guard.check())?;
        let host = self
            .scan_worker
            .as_ref()
            .ok_or(BusinessError::Unsupported)?;
        let deadline = scan_started
            .checked_add(Duration::from_millis(self.scan_budget.max_duration_ms))
            .ok_or(BusinessError::BudgetExceeded)?;
        let runtime = crate::scan_worker_runtime::ScanWorkerRuntime::new(host, deadline);
        // 仅可信本地诊断显式开启；固定阶段不携带路径、主体、job 或正文。
        let timing =
            std::env::var_os("DISKGRAPH_SCAN_DIAGNOSTICS").is_some_and(|value| value == "1");
        let trace = |phase: &str, nodes: u64| {
            if timing {
                eprintln!(
                    "diskgraph: scan_phase={phase} nodes={nodes} elapsed_ms={}",
                    scan_started.elapsed().as_millis()
                );
            }
        };
        trace("worker_begin", 0);
        let mut scanner_observed = false;
        let tree = runtime.run(
            &root,
            &options,
            &mut || observation_guard.check(),
            &mut || {
                if now_ms().saturating_sub(last_heartbeat) >= 5000 {
                    let heartbeat =
                        self.control()?
                            .heartbeat_fenced(job_id, owner, job.fencing_token);
                    #[cfg(test)]
                    crate::job_stop_cause_hooks::heartbeat_checked(job_id, &heartbeat);
                    heartbeat?;
                    last_heartbeat = now_ms();
                }
                let checked = observation_guard.check_now();
                #[cfg(test)]
                crate::job_stop_cause_hooks::scan_checked(job_id, &checked);
                if checked.is_err() {
                    cancel.store(true, Ordering::SeqCst);
                }
                checked?;
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
                    return Err(stop_reason
                        .take()
                        .unwrap_or_else(|| BusinessError::Conflict.into()));
                }
                Ok(())
            },
            &mut |progress| {
                if !scanner_observed {
                    scanner_observed = true;
                    // 第一份真实Progress来自helper内ScanHandle::spawn之后；不把fork/Hello当walk已启动。
                    #[cfg(test)]
                    crate::job_stop_cause_hooks::after_spawn(job_id);
                }
                if let Ok(mut entries) = self.scan_progress.lock() {
                    entries.insert(
                        (job_id.to_owned(), job.fencing_token),
                        progress.clone().into_native(),
                    );
                }
                if progress.files.saturating_add(progress.dirs)
                    > self.max_nodes_per_scan.min(self.scan_budget.max_nodes)
                    || scan_started.elapsed()
                        > Duration::from_millis(self.scan_budget.max_duration_ms)
                {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                Ok(())
            },
        )?;
        trace("worker_complete", 0);
        let scanned = diskgraph_disktree::convert_tree(&root, &tree, scan_settings(&options))
            .map_err(|error| {
                if error.kind() == io::ErrorKind::Unsupported {
                    EngineError::Business(BusinessError::Unsupported)
                } else {
                    EngineError::Io(error)
                }
            })?;
        drop(tree);
        trace("conversion_complete", scanned.nodes.len() as u64);
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

        let mut observation_time = Duration::ZERO;
        let mut staging_write_time = Duration::ZERO;
        // 采样和编码发生于数据库写锁外，额外对象只保留当前配置批次。
        for batch in scanned
            .nodes
            .chunks(self.scan_budget.write_batch_nodes.max(1) as usize)
        {
            observation_guard.check_now()?;
            if !self.accepts_new_work() {
                return Err(BusinessError::ResourceExhausted.into());
            }
            let observation_started = timing.then(Instant::now);
            #[cfg(windows)]
            let observations = crate::scan_observation_batch::collect_ordered(batch, &|chunk| {
                // 门禁局部状态不能跨线程共享；原期限、授权与取消标记保持同一执行代次。
                // keeper原错误只由主执行器消费，采样线程不取走其所有权。
                let guard =
                    ScanObservationGuard::new(self, &job, authority.as_ref(), cancel, scan_started);
                chunk
                    .iter()
                    .map(|node| {
                        guard.check()?;
                        let locator = qualify_scan_locator(node)?;
                        let observed = native_root.observe(
                            &locator
                                .to_native_path()
                                .map_err(|_| BusinessError::Unsupported)?,
                            &node.v1,
                            node.self_modified,
                            &scanned.snapshot.settings,
                            &|| guard.check(),
                        )?;
                        Ok((locator, observed))
                    })
                    .collect::<Result<Vec<_>, EngineError>>()
            });
            #[cfg(windows)]
            observation_guard.check_now()?;
            #[cfg(windows)]
            let mut observations = observations?.into_iter();
            let mut prepared_nodes = Vec::with_capacity(batch.len());
            #[cfg(target_os = "linux")]
            let mut unix_observations = Vec::with_capacity(batch.len());
            for node in batch {
                observation_guard.check()?;
                #[cfg(not(windows))]
                let locator = qualify_scan_locator(node)?;
                #[cfg(windows)]
                let (locator, observed) = observations
                    .next()
                    .expect("one observation per successfully sampled node");
                #[cfg(not(windows))]
                let observed = (None, Some(WindowsObservationGap::Unsupported));
                #[cfg(target_os = "linux")]
                let unix_observed = if node.v1.kind == diskgraph_core::NodeKind::File
                    && !node.v1.read_error
                {
                    let observed = match &unix_root {
                        Ok(root) => root.observe(
                            node,
                            &locator
                                .to_native_path()
                                .map_err(|_| BusinessError::Unsupported)?,
                            &|| observation_guard.check(),
                        )?,
                        Err(error) => (None, Some(crate::native_process::linux_scan_gap(*error))),
                    };
                    Some(observed)
                } else {
                    None
                };
                #[cfg(test)]
                crate::scan_observation_tests::after_observe(&job.job_id);
                #[cfg(test)]
                crate::job_authorization_tests::after_observe(&job.job_id);
                observation_guard.check()?;
                usage.elapsed_ms =
                    u64::try_from(scan_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let prepared = PreparedStagingNode::new(
                    &node.v1,
                    Some(locator),
                    node.self_modified,
                    observed.0.as_ref(),
                    observed.1,
                )?;
                let cost = prepared.encoded_cost();
                // 与旁表写入共用编码校验，按实际元数据字节计费，保留Core编码上限。
                #[cfg(target_os = "linux")]
                let cost = cost
                    .checked_add(
                        unix_observed
                            .as_ref()
                            .map(|(observation, gap)| {
                                diskgraph_store::staging_unix_observation_encoded_cost(
                                    observation.as_ref(),
                                    *gap,
                                )
                            })
                            .transpose()?
                            .unwrap_or(0),
                    )
                    .ok_or(BusinessError::BudgetExceeded)?;
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
                prepared_nodes.push(prepared);
                #[cfg(target_os = "linux")]
                unix_observations.push(unix_observed);
            }
            if let Some(started) = observation_started {
                observation_time += started.elapsed();
            }
            let staging_write_started = timing.then(Instant::now);
            observation_guard.check_now()?;
            // 原生调用已结束；遵循既有 graph→control 写锁顺序，并在原子 fence 下暂存。
            #[cfg(test)]
            crate::scan_observation_tests::before_stage_lock(&job.job_id);
            let mut graph = self.graph()?;
            let mut control = self.control()?;
            let lease_expires = control.job(job_id)?.lease_expires_unix_ms;
            control.with_job_fence(job_id, owner, job.fencing_token, || {
                graph.append_prepared_staging_iter_checked(
                    &staging_id,
                    prepared_nodes.iter(),
                    || {
                        crate::job_authorization::check_scan_commit(
                            authority.as_ref(),
                            cancel,
                            lease_expires,
                            scan_started,
                            self.scan_budget.max_duration_ms,
                        )
                    },
                )?;
                #[cfg(target_os = "linux")]
                graph.append_staging_unix_observations_checked(
                    &staging_id,
                    batch
                        .iter()
                        .zip(&unix_observations)
                        .filter_map(|(node, observation)| {
                            observation
                                .as_ref()
                                .map(|(value, gap)| (node.v1.id, value.as_ref(), *gap))
                        }),
                    || {
                        crate::job_authorization::check_scan_commit(
                            authority.as_ref(),
                            cancel,
                            lease_expires,
                            scan_started,
                            self.scan_budget.max_duration_ms,
                        )
                    },
                )?;
                Ok(())
            })?;
            if let Some(started) = staging_write_started {
                staging_write_time += started.elapsed();
            }
            trace("staging_batch_complete", usage.nodes);
        }
        trace("staging_complete", usage.nodes);
        #[cfg(windows)]
        native_root.emit_cost_diagnostic(usage.nodes);
        if timing {
            crate::scan_cost_diagnostic::emit(observation_time, staging_write_time, usage.nodes);
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
        #[cfg(target_os = "linux")]
        if let Ok(root) = &unix_root {
            root.validate(&|| observation_guard.check())?;
        }
        #[cfg(test)]
        crate::job_stop_cause_hooks::after_final_scan_validation(job_id);
        // 先在数据库锁外等待原安装锁；守卫跨越完整发布事务，更新不能穿过检查/提交窗口。
        #[cfg(target_os = "macos")]
        let _installation_publication_guard =
            host.authorize_macos_epoch(deadline, &mut || observation_guard.check_now())?;
        let mut graph = self.graph()?;
        if !self.accepts_new_work() {
            graph.clear_staging(&staging_id)?;
            return Err(EngineError::Business(BusinessError::ResourceExhausted));
        }
        // 撤权和发布共用控制库锁，防止检查通过后撤权仍发布。
        let mut control = self.control()?;
        if control.scope(&job.scope_id)?.revoked {
            graph.clear_staging(&staging_id)?;
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        if cancel.load(Ordering::SeqCst) {
            graph.clear_staging(&staging_id)?;
            // 同代 keeper 已先保存原因再置停止位；此处不把真实 SQL 失败改成撤权。
            // 没有 keeper 原因的旧取消路径保留原返回，实际 scope 撤销优先拒权。
            return Err(stop_reason
                .take()
                .unwrap_or_else(|| BusinessError::PermissionDenied.into()));
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
        let lease_expires = control.job(job_id)?.lease_expires_unix_ms;
        let receipt = diskgraph_store::ScanPublicationReceipt::new(
            &job,
            server_id.clone(),
            authority.clone(),
            observed_graph.snapshot.id.clone(),
            revision_id.clone(),
            published_at,
        )?;
        trace("publication_begin", usage.nodes);
        let result = control.with_job_fence(job_id, owner, job.fencing_token, || {
            let elapsed = scan_started.elapsed();
            #[cfg(test)]
            let elapsed = crate::scan_publication_tests::publication_elapsed(job_id, elapsed);
            // 包括转换、暂存、项目采集和锁等待；提交前再次协作检查整次扫描期限。
            if elapsed > Duration::from_millis(self.scan_budget.max_duration_ms) {
                return Err(StoreError::BudgetExceeded);
            }
            graph.publish_scan_revision_checked(
                &staging_id,
                &observed_graph,
                &receipt,
                Some(&collector),
                || {
                    crate::job_authorization::check_scan_commit(
                        authority.as_ref(),
                        cancel,
                        lease_expires,
                        scan_started,
                        self.scan_budget.max_duration_ms,
                    )
                },
            )
        });
        drop(control);
        if let Err(error) = result {
            let _ = graph.clear_staging(&staging_id);
            return Err(error.into());
        }

        trace("publication_complete", usage.nodes);
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
