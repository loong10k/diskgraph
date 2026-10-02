//! 共享 Engine 的 engine_startup 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{ResourceLocator, ServerId};
use diskgraph_store::{ControlStore, SqliteSnapshotStore};
use std::collections::HashMap;
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
        let roots = control
            .list_scopes()?
            .into_iter()
            .map(|scope| {
                (
                    scope.scope_id.as_str().to_owned(),
                    match scope.root.kind {
                        diskgraph_core::LocatorKind::NativePath => {
                            ResourceLocator::NativePath(scope.root.display().to_owned())
                        }
                        diskgraph_core::LocatorKind::DocumentUri => {
                            ResourceLocator::DocumentUri(scope.root.display().to_owned())
                        }
                    },
                )
            })
            .collect::<Vec<_>>();
        graph.backfill_revision_ownership(server_id.as_str(), &roots)?;
        Ok(Self {
            data_dir: config.data_dir,
            graph_path,
            max_nodes_per_scan: config.max_nodes_per_scan,
            scan_budget: config.scan_budget,
            capacity_watermark: config.capacity_watermark,
            max_active_jobs_per_principal: config.max_active_jobs_per_principal,
            scan_options: config.scan_options.clone(),
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
