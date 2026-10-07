//! 共享 Engine 的 engine_startup 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{ResourceLocator, ServerId};
use diskgraph_store::{ControlStore, SqliteSnapshotStore};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

impl Engine {
    /// 创建并打开现有数据布局，备份升级并回填可确认的 revision 归属。
    /// 参数：config 指定数据库路径、容量、作业与扫描配置。
    /// 返回：唯一引擎实例或升级/存储/权限失败；不启动 runner。
    /// Opens (creating if needed) the engine's data directory with both stores.
    pub fn open(config: EngineConfig) -> Result<Self, EngineError> {
        std::fs::create_dir_all(&config.data_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&config.data_dir)?.permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(&config.data_dir, permissions)?;
        }
        let graph_path = config
            .graph_database_path
            .clone()
            .unwrap_or_else(|| config.data_dir.join("diskgraph.sqlite"));
        let (mut graph, _) = SqliteSnapshotStore::open_with_backup(
            &graph_path,
            &config.data_dir.join("migration_backups"),
        )?;
        let mut control = ControlStore::open(&config.data_dir.join("diskgraph-control.sqlite"))?;
        let server_id = control.ensure_server()?;
        let mut unverifiable_roots = HashSet::new();
        let scopes = control.list_scopes()?;
        let mut roots = scopes
            .iter()
            .map(|scope| {
                let root = match scope.root.kind {
                    diskgraph_core::LocatorKind::NativePath => {
                        ResourceLocator::NativePath(scope.root.display().to_owned())
                    }
                    diskgraph_core::LocatorKind::DocumentUri => {
                        ResourceLocator::DocumentUri(scope.root.display().to_owned())
                    }
                };
                // 旧快照仅有字符串：先证明注册根的原始身份能无损表示为该字符串。
                // 非 UTF-8、外平台不可解码或损坏定位不能凭显示别名获得旧 revision。
                let lossless = match scope.root.kind {
                    diskgraph_core::LocatorKind::NativePath => scope
                        .root
                        .to_native_path()
                        .ok()
                        .and_then(|path| path.to_str().map(str::to_owned))
                        .is_some_and(|path| path == scope.root.display()),
                    diskgraph_core::LocatorKind::DocumentUri => scope
                        .root
                        .raw_bytes()
                        .ok()
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                        .is_some_and(|uri| uri == scope.root.display()),
                };
                if !lossless {
                    unverifiable_roots.insert(root.clone());
                }
                (scope.scope_id.as_str().to_owned(), root)
            })
            .collect::<Vec<_>>();
        // 拒绝整个有损别名组，不能删除冲突候选后人为制造“唯一匹配”。
        for scope in &scopes {
            let display_root = match scope.root.kind {
                diskgraph_core::LocatorKind::NativePath => {
                    ResourceLocator::NativePath(scope.root.display().to_owned())
                }
                diskgraph_core::LocatorKind::DocumentUri => {
                    ResourceLocator::DocumentUri(scope.root.display().to_owned())
                }
            };
            if unverifiable_roots.contains(&display_root) {
                let native = match scope.root.kind {
                    diskgraph_core::LocatorKind::NativePath => {
                        scope.root.to_native_path().ok().and_then(|path| {
                            diskgraph_core::QualifiedLocator::from_native_path(&path).ok()
                        })
                    }
                    diskgraph_core::LocatorKind::DocumentUri => scope
                        .root
                        .raw_bytes()
                        .ok()
                        .and_then(|raw| String::from_utf8(raw).ok())
                        .and_then(|uri| {
                            diskgraph_core::QualifiedLocator::from_document_uri(uri).ok()
                        }),
                };
                graph.isolate_unconfirmed_revision_roots(
                    server_id.as_str(),
                    scope.scope_id.as_str(),
                    native.as_ref(),
                )?;
            }
        }
        roots.retain(|(_, root)| !unverifiable_roots.contains(root));
        graph.backfill_revision_ownership(server_id.as_str(), &roots)?;
        Ok(Self {
            data_dir: config.data_dir,
            graph_path,
            max_nodes_per_scan: config.max_nodes_per_scan,
            scan_budget: config.scan_budget,
            capacity_watermark: config.capacity_watermark,
            max_active_jobs_per_principal: config.max_active_jobs_per_principal,
            scan_options: config.scan_options.clone(),
            scan_worker: None,
            #[cfg(windows)]
            probe_host: None,
            #[cfg(windows)]
            runner_admission: Mutex::new(()),
            graph: Mutex::new(graph),
            control: Mutex::new(control),
            cancellations: Mutex::new(HashMap::new()),
            scan_progress: Mutex::new(HashMap::new()),
        })
    }
}

impl Engine {
    /// 返回用于诊断的数据目录。
    /// 参数：无。
    /// 返回：借用的数据目录路径。
    /// Where both databases live (diagnostics only).
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

impl Engine {
    /// 读取并按既有规则确保持久 server 身份。
    /// 参数：无。
    /// 返回：server ID 或控制库失败。
    /// Mints once and then serves the persistent server identity (ST-05).
    pub fn server_id(&self) -> Result<ServerId, EngineError> {
        Ok(self.control()?.ensure_server()?)
    }
}

impl Engine {
    /// 参数：config保留旧两库/扫描配置，host为普通Rust独立受信镜像/响应额度/有限容量。
    /// 返回：原Engine与必须在catch_unwind外保存的唯一Recovery；不启动runner或授予请求权限。
    /// 旧open签名不变；服务Drop不把未回收槽清空，宿主必须join runner并实际drain至完成。
    pub fn open_with_scan_worker(
        config: EngineConfig,
        host: crate::ScanWorkerHost,
    ) -> Result<(Self, crate::ScanWorkerRecovery), EngineError> {
        let mut engine = Self::open(config)?;
        let recovery = crate::ScanWorkerRecovery::new(std::sync::Arc::clone(&host.registry));
        engine.scan_worker = Some(std::sync::Arc::new(host));
        Ok((engine, recovery))
    }
}

#[cfg(windows)]
impl Engine {
    /// 参数：config为原两库配置，scan为可选受信扫描镜像，probe为固定容量探针宿主。
    /// 返回：引擎与扫描恢复责任；调用方在本函数前已持有独立ProbeRecovery，不能放入请求catch。
    /// 宿主材料不授予请求权限；没有扫描镜像仍可使用受管理证据探针。
    pub fn open_with_process_hosts(
        config: EngineConfig,
        scan: Option<crate::ScanWorkerHost>,
        probe: crate::ProbeHost,
    ) -> Result<(Self, Option<crate::ScanWorkerRecovery>), EngineError> {
        let (mut engine, recovery) = match scan {
            Some(host) => {
                let (engine, recovery) = Self::open_with_scan_worker(config, host)?;
                (engine, Some(recovery))
            }
            None => (Self::open(config)?, None),
        };
        engine.probe_host = Some(probe);
        Ok((engine, recovery))
    }
}
