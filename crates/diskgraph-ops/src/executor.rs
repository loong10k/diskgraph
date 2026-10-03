//! executor：既有文件操作职责的原生 Rust 实现。
use diskgraph_engine::Engine;

/// 唯一文件操作执行状态对象，持有原 Engine 和可选永久删除批准方。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::Executor`，保留既有语义。
/// Executes a plan. This is the only place a file moves, and only after the
/// approval verifies and every precondition is rechecked against the live
/// filesystem.
pub struct Executor {
    pub(super) engine: std::sync::Arc<Engine>,
    /// The trusted surface whose approvals may authorize a purge. `None`
    /// (the default) means purge is disabled outright: nothing in this build
    /// can be removed permanently until an operator names the authority (OP-07).
    pub(super) purge_authority: Option<String>,
}

impl Executor {
    /// 创建原有状态对象。
    /// 参数：engine 为授权、控制记录和文件动作使用的共享引擎。
    /// 返回：持有相同依赖的新对象。
    pub fn new(engine: std::sync::Arc<Engine>) -> Self {
        Self {
            engine,
            purge_authority: None,
        }
    }

    /// 设置唯一允许永久删除的可信批准方。
    /// 参数：authority 为宿主指定的批准方名称。
    /// 返回：更新同一执行器的配置；不启用 CLI/MCP 工具。
    /// Names the only approval issuer whose approvals may authorize purge.
    /// Any other issuer's approval for a purge plan is refused at apply time.
    pub fn with_purge_authority(mut self, authority: &str) -> Self {
        self.purge_authority = Some(authority.to_owned());
        self
    }
}
