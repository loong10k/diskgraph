//! MCP 生命周期与唯一共享状态；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::protocol::ToolProfile;
use crate::{McpConfig, request_context};
use diskgraph_engine::{Engine, EngineConfig, EngineError};

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

    /// 读取服务启用的工具集合。
    /// 参数：无。返回：当前工具 profile。
    pub fn profile(&self) -> ToolProfile {
        self.profile
    }
}
