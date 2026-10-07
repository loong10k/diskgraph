//! MCP 生命周期与唯一共享状态；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::protocol::ToolProfile;
use crate::{McpConfig, request_context};
use diskgraph_engine::{Engine, EngineConfig, EngineError, ScanWorkerHost, ScanWorkerRecovery};

/// 共享唯一 Engine 的服务状态；来源：原生 Rust diskgraph-mcp::McpService。
/// crate 内可见性延续原根模块对子模块开放的边界，不开放新的公共字段。
#[derive(Clone)]
pub struct McpService {
    pub(crate) engine: std::sync::Arc<Engine>,
    pub(crate) context: request_context::RequestContext,
    pub(crate) profile: ToolProfile,
    pub(crate) legacy_sse: bool,
    pub(crate) initialized: bool,
}

impl McpService {
    /// 打开本地 Engine 并按 CLI 相同策略引导 stdio 主体。
    /// 参数：config 为本地服务配置。返回：服务或引擎错误。
    pub fn open(config: McpConfig) -> Result<Self, EngineError> {
        Self::open_mode(config, true)
    }

    /// 打开远程 Engine，不创建本地管理员授权。
    /// 参数：config 为远程服务配置。返回：服务或引擎错误。
    pub fn open_remote(config: McpConfig) -> Result<Self, EngineError> {
        Self::open_mode(config, false)
    }

    /// 使用独立受信宿主材料打开本地 stdio 服务，并延续原本地管理引导。
    /// 来源：原生 Rust PF-06 显式宿主与外部恢复句柄契约。
    /// 参数：config 为本地配置，host 为宿主提供的 held 镜像、独立预期及有限容量。
    /// 返回：服务与唯一 Recovery；宿主必须在 catch_unwind 外保留它，停止 runner 后实际 drain。
    pub fn open_with_scan_worker(
        config: McpConfig,
        host: ScanWorkerHost,
    ) -> Result<(Self, ScanWorkerRecovery), EngineError> {
        Self::open_scan_worker_mode(config, host, true)
    }

    /// 使用独立受信宿主材料打开远程服务，不创建本地管理员或继承本地请求能力。
    /// 来源：原生 Rust PF-06；镜像材料不授予客户端任何请求权限。
    /// 参数：config 为远程配置，host 为可信宿主材料，不来自工具 JSON 或邻接清单。
    /// 返回：服务与必须在 catch_unwind 外保留的唯一 Recovery；服务 clone 不复制恢复责任。
    pub fn open_remote_with_scan_worker(
        config: McpConfig,
        host: ScanWorkerHost,
    ) -> Result<(Self, ScanWorkerRecovery), EngineError> {
        Self::open_scan_worker_mode(config, host, false)
    }

    /// 参数：config/host 为显式本地配置和宿主材料，trusted_local 选择原请求信任边界。
    /// 返回：共享唯一 Engine 的服务与外部恢复句柄；此启动路径不创建物理 child。
    fn open_scan_worker_mode(
        config: McpConfig,
        host: ScanWorkerHost,
        trusted_local: bool,
    ) -> Result<(Self, ScanWorkerRecovery), EngineError> {
        let (engine, recovery) = Engine::open_with_scan_worker(
            EngineConfig {
                data_dir: config.data_dir,
                max_nodes_per_scan: 2_000_000,
                ..EngineConfig::default()
            },
            host,
        )?;
        if trusted_local {
            engine.bootstrap_local_admin(&config.principal)?;
        }
        let service = Self {
            engine: std::sync::Arc::new(engine),
            context: if trusted_local {
                request_context::RequestContext::local(config.principal)
            } else {
                request_context::RequestContext::unauthenticated(config.principal)
            },
            profile: config.profile,
            legacy_sse: config.legacy_sse,
            initialized: false,
        };
        Ok((service, recovery))
    }

    /// 建立唯一共享 Engine，并选择可信本地或未认证请求来源。
    /// 参数：config 为配置，trusted_local 选择启动边界。返回：服务或引擎错误。
    fn open_mode(config: McpConfig, trusted_local: bool) -> Result<Self, EngineError> {
        let engine = Engine::open(EngineConfig {
            data_dir: config.data_dir,
            max_nodes_per_scan: 2_000_000,
            ..EngineConfig::default()
        })?;
        if trusted_local {
            engine.bootstrap_local_admin(&config.principal)?;
        }
        Ok(Self {
            engine: std::sync::Arc::new(engine),
            context: if trusted_local {
                request_context::RequestContext::local(config.principal)
            } else {
                request_context::RequestContext::unauthenticated(config.principal)
            },
            profile: config.profile,
            legacy_sse: config.legacy_sse,
            initialized: false,
        })
    }

    /// Windows受管理进程宿主入口；来源：原生 Rust PF-06，无 Java 对等对象。
    /// 参数：config 为服务配置，scan 可选扫描宿主，probe 为固定容量探针宿主；
    /// trusted_local 仅由启动传输选择，远程服务必须传 false，不能由工具输入控制。
    /// 返回：服务及可选扫描恢复责任；调用者必须事先在 catch 外保留探针 Recovery。
    #[cfg(windows)]
    pub fn open_with_process_hosts(
        config: McpConfig,
        scan: Option<ScanWorkerHost>,
        probe: diskgraph_engine::ProbeHost,
        trusted_local: bool,
    ) -> Result<(Self, Option<ScanWorkerRecovery>), EngineError> {
        let (engine, recovery) = Engine::open_with_process_hosts(
            EngineConfig {
                data_dir: config.data_dir,
                max_nodes_per_scan: 2_000_000,
                ..EngineConfig::default()
            },
            scan,
            probe,
        )?;
        if trusted_local {
            engine.bootstrap_local_admin(&config.principal)?;
        }
        Ok((
            Self {
                engine: std::sync::Arc::new(engine),
                context: if trusted_local {
                    request_context::RequestContext::local(config.principal)
                } else {
                    request_context::RequestContext::unauthenticated(config.principal)
                },
                profile: config.profile,
                legacy_sse: config.legacy_sse,
                initialized: false,
            },
            recovery,
        ))
    }

    /// 借用当前服务共享的 Engine。
    /// 参数：无。返回：同一个 Engine 的借用。
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// 启动独立于客户端连接的后台持久任务执行器。宿主保留句柄，测试可显式控制。
    /// 参数：无。返回：按本地或远程信任边界选择的任务执行器句柄。
    pub fn start_job_runner(&self) -> diskgraph_engine::JobRunner {
        if self.context.trusted_local() {
            diskgraph_engine::JobRunner::start(std::sync::Arc::clone(&self.engine))
        } else {
            diskgraph_engine::JobRunner::start_strict(std::sync::Arc::clone(&self.engine))
        }
    }

    /// 用同一外部Recovery启动任务执行器，每轮新认领前尝试一次原资源恢复。
    /// 来源：原生 Rust PF-06；参数：recovery 为协议 catch 外保留的原责任，返回：按请求信任边界调度的 runner。
    #[cfg(windows)]
    pub fn start_job_runner_with_probe_recovery(
        &self,
        recovery: std::sync::Arc<diskgraph_engine::ProbeRecovery>,
    ) -> Result<diskgraph_engine::JobRunner, EngineError> {
        diskgraph_engine::JobRunner::start_with_probe_recovery(
            std::sync::Arc::clone(&self.engine),
            !self.context.trusted_local(),
            recovery,
        )
    }

    /// 消费原宿主服务并交给实际退休 owner；不会复制扫描恢复责任或清除槽。
    /// 参数：scan/slot 为同次启动的原恢复责任和 ACTIVE 槽，runner 须已停止并 join。
    /// 返回：保留原 Engine 的完整材料；绑定验证由 SupervisorOwner 执行，非远程工具入口。
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn into_supervisor_parts(
        self,
        scan: Option<ScanWorkerRecovery>,
        slot: diskgraph_engine::recovery_slot::ActiveSlot,
    ) -> diskgraph_engine::SupervisorParts {
        diskgraph_engine::SupervisorParts {
            engine: self.engine,
            scan,
            slot,
        }
    }

    /// 读取服务启用的工具集合。
    /// 参数：无。返回：当前工具 profile。
    pub fn profile(&self) -> ToolProfile {
        self.profile
    }
}
