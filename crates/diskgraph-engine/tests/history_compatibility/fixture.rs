//! D35 合法旧观测导入夹具；只通过公开 Store API 发布，不修改已注册 scope。

mod request_authorizer;

use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, FileIdentity, NodeKind, PrincipalId, QueryBudget,
    ResourceLocator, ScanCoverage, ScanSettings, ScopeId,
};
// 三桌面扫描夹具显式持有受信宿主和原恢复责任，保留原业务断言。
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "../support/native_scan_engine.rs"]
mod native_scan_engine;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use diskgraph_engine::Engine;
use diskgraph_engine::{ComparisonReport, EngineConfig, RevisionGrowth};
use diskgraph_store::{Result as StoreResult, SqliteSnapshotStore};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use native_scan_engine::NativeScanEngine as Engine;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 实际 Engine/scope 与明确标识为合成的导入历史；来源：DiskGraph 原生 Rust D35 / Q-04 测试。
pub(crate) struct HistoryFixture {
    pub(crate) engine: Engine,
    pub(crate) directory: tempfile::TempDir,
    pub(crate) root: PathBuf,
    pub(crate) principal: PrincipalId,
    pub(crate) scope: ScopeId,
}

impl HistoryFixture {
    /// 参数：无；返回：真实注册范围及独立数据库，未生成任何导入历史。
    pub(crate) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            scan_options: diskgraph_disktree_core::scan::ScanOptions {
                apparent_size: true,
                dedup_hardlinks: false,
                ..Default::default()
            },
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("history-matrix").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        Self {
            directory,
            root,
            engine,
            principal,
            scope,
        }
    }

    /// 参数：id/time/bytes 为合成快照属性；返回：同真实注册根的合成旧图，不声称来自原生扫描。
    pub(crate) fn graph(&self, id: &str, time: u64, bytes: u64) -> DiskGraph {
        let locator = ResourceLocator::NativePath(self.root.to_str().unwrap().to_owned());
        let root = DiskNode {
            id: 1,
            parent_id: None,
            locator: locator.clone(),
            name: "root".into(),
            kind: NodeKind::Directory,
            subtree_bytes: bytes,
            direct_bytes: 0,
            size_known: true,
            files: 1,
            directories: 1,
            modified_unix_seconds: Some(100),
            file_identity: None,
            category_hint: None,
            reclaim_hint: None,
            read_error: false,
        };
        let file = DiskNode {
            id: 2,
            parent_id: Some(1),
            locator: ResourceLocator::NativePath(self.root.join("item").to_str().unwrap().into()),
            name: "item".into(),
            kind: NodeKind::File,
            subtree_bytes: bytes,
            direct_bytes: bytes,
            directories: 0,
            file_identity: Some(FileIdentity {
                volume_id: "synthetic-import-volume".into(),
                file_id: 42,
            }),
            ..root.clone()
        };
        DiskGraph {
            snapshot: DiskSnapshot {
                id: id.into(),
                root: locator,
                volume_id: Some("synthetic-import-volume".into()),
                captured_at_unix_ms: time,
                settings: ScanSettings {
                    apparent_size: true,
                    follow_links: false,
                    include_hidden: true,
                    one_filesystem: true,
                    max_depth: None,
                    dedup_hardlinks: false,
                },
                coverage: ScanCoverage {
                    complete: true,
                    unreadable_nodes: 0,
                    depth_limited: false,
                },
            },
            nodes: vec![root, file],
            evidence: Vec::new(),
        }
    }

    /// 参数：graph 为合法旧观测；返回：公开暂存/发布的结果，包含实际 server/scope 归属。
    pub(crate) fn publish(&self, graph: &DiskGraph) -> StoreResult<()> {
        let mut store =
            SqliteSnapshotStore::open(&self.directory.path().join("data/diskgraph.sqlite"))?;
        store.append_staging_nodes(&graph.snapshot.id, &graph.nodes)?;
        store.publish_revision_owned(
            &graph.snapshot.id,
            graph,
            &graph.snapshot.id,
            graph.snapshot.captured_at_unix_ms,
            Some((
                self.engine.server_id().unwrap().as_str(),
                self.scope.as_str(),
            )),
        )
    }

    /// 参数：左右 revision 与相对路径；返回：公开授权增长，测试失败保留实际错误。
    pub(crate) fn growth(&self, left: &str, right: &str, path: &str) -> Option<RevisionGrowth> {
        let authorizer =
            request_authorizer::RequestAuthorizer::new(self.engine.policy_authorizer().unwrap());
        let started = Instant::now();
        let result = self.engine.growth_between_until(
            left,
            right,
            Path::new(path),
            QueryBudget::default(),
            &self.principal,
            &authorizer,
            deadline(),
        );
        if let Err(error) = &result {
            authorizer.report();
            eprintln!(
                "HISTORY_GROWTH_DIAGNOSTIC elapsed_ms={} error={error:?}",
                started.elapsed().as_secs_f64() * 1000.0,
            );
        }
        result.unwrap()
    }

    /// 参数：左右 revision；返回：公开授权变化 JSON。
    pub(crate) fn changes(&self, left: &str, right: &str) -> serde_json::Value {
        let authorizer =
            request_authorizer::RequestAuthorizer::new(self.engine.policy_authorizer().unwrap());
        let started = Instant::now();
        let result = self.engine.revision_changes_until(
            left,
            right,
            QueryBudget::default(),
            &self.principal,
            &authorizer,
            deadline(),
        );
        if let Err(error) = &result {
            authorizer.report();
            eprintln!(
                "HISTORY_CHANGES_DIAGNOSTIC elapsed_ms={} configured_budget_ms={} business_budget_exceeded={}",
                started.elapsed().as_secs_f64() * 1000.0,
                QueryBudget::default().deadline_ms,
                matches!(
                    error,
                    diskgraph_engine::EngineError::Business(
                        diskgraph_core::BusinessError::BudgetExceeded
                    )
                )
            );
        }
        result.unwrap()
    }

    /// 参数：左右 revision；返回：独立的通用授权元数据比较报告，不调用内容读取。
    pub(crate) fn compare(&self, left: &str, right: &str) -> ComparisonReport {
        let authorizer =
            request_authorizer::RequestAuthorizer::new(self.engine.policy_authorizer().unwrap());
        let started = Instant::now();
        let result = self.engine.compare_revisions_until(
            left,
            right,
            0,
            QueryBudget::default(),
            &self.principal,
            &authorizer,
            deadline(),
        );
        if let Err(error) = &result {
            authorizer.report();
            eprintln!(
                "HISTORY_COMPARE_DIAGNOSTIC elapsed_ms={} error={error:?}",
                started.elapsed().as_secs_f64() * 1000.0,
            );
        }
        result.unwrap()
    }

    /// 参数：无；返回：当前真实根的完整扫描 revision，用于与合成矩阵区分的宿主证据。
    pub(crate) fn native_scan(&self) -> String {
        let job = self
            .engine
            .sync_scope(
                &self.scope,
                &self.principal,
                &self.engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        self.engine
            .run_job(&job.job_id, "history-matrix-native")
            .unwrap();
        self.engine.latest_revision(&self.scope).unwrap().unwrap()
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
