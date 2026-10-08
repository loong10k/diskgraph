//! 共享 Engine 的 engine_startup 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::ServerId;
use diskgraph_store::{ControlStore, SqliteSnapshotStore};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

impl Engine {
    /// 沿原绝对启动期限打开引擎。参数：config为原配置，deadline为调用方原期限。
    /// 返回：期限内实例或预算错误；同步存储操作不提供硬抢占。
    pub fn open_until(
        config: EngineConfig,
        deadline: std::time::Instant,
    ) -> Result<Self, EngineError> {
        Self::open_checked(config, &|| {
            if std::time::Instant::now() >= deadline {
                Err(diskgraph_core::BusinessError::BudgetExceeded.into())
            } else {
                Ok(())
            }
        })
    }

    /// 创建并打开现有数据布局，备份升级并回填可确认的 revision 归属。
    /// 参数：config 指定数据库路径、容量、作业与扫描配置。
    /// 返回：唯一引擎实例或升级/存储/权限失败；不启动 runner。
    /// Opens (creating if needed) the engine's data directory with both stores.
    pub fn open(config: EngineConfig) -> Result<Self, EngineError> {
        Self::open_checked(config, &|| Ok(()))
    }

    fn open_checked(
        config: EngineConfig,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        check()?;
        std::fs::create_dir_all(&config.data_dir)?;
        check()?;
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
        check()?;
        let (mut graph, _) = SqliteSnapshotStore::open_with_backup(
            &graph_path,
            &config.data_dir.join("migration_backups"),
        )?;
        check()?;
        let mut control = ControlStore::open(&config.data_dir.join("diskgraph-control.sqlite"))?;
        check()?;
        let server_id = control.ensure_server()?;
        check()?;
        let roots = crate::revision_root_reconciliation::eligible_roots(
            &mut graph,
            &server_id,
            &control.list_scopes()?,
        )?;
        check()?;
        graph.backfill_revision_ownership(server_id.as_str(), &roots)?;
        // 同步迁移/回填可能越过原期限；不得向调用方交出迟到实例。
        check()?;
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
    /// 参数：config与host沿用原契约，deadline为原启动期限。
    /// 返回：期限内引擎和恢复责任；未启动任何worker，不刷新期限。
    pub fn open_with_scan_worker_until(
        config: EngineConfig,
        host: crate::ScanWorkerHost,
        deadline: std::time::Instant,
    ) -> Result<(Self, crate::ScanWorkerRecovery), EngineError> {
        let mut engine = Self::open_until(config, deadline)?;
        let recovery = crate::ScanWorkerRecovery::new(std::sync::Arc::clone(&host.registry));
        engine.scan_worker = Some(std::sync::Arc::new(host));
        Ok((engine, recovery))
    }

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

#[cfg(windows)]
impl Engine {
    /// 参数：config、scan和probe为原宿主配置，deadline为原启动期限。
    /// 返回：期限内引擎和可选扫描恢复责任；不启动原生任务。
    pub fn open_with_process_hosts_until(
        config: EngineConfig,
        scan: Option<crate::ScanWorkerHost>,
        probe: crate::ProbeHost,
        deadline: std::time::Instant,
    ) -> Result<(Self, Option<crate::ScanWorkerRecovery>), EngineError> {
        let (mut engine, recovery) = match scan {
            Some(host) => {
                let (engine, recovery) = Self::open_with_scan_worker_until(config, host, deadline)?;
                (engine, Some(recovery))
            }
            None => (Self::open_until(config, deadline)?, None),
        };
        engine.probe_host = Some(probe);
        Ok((engine, recovery))
    }
}

#[cfg(test)]
#[path = "engine_startup_tests.rs"]
mod tests;
