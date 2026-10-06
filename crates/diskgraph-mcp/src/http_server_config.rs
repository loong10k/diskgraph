use crate::http::Security;
use crate::http_limits::HttpLimits;

/// 单个 HTTP listener 的资源、安全和旧 SSE 适配开关配置。
/// 来源：原生 Rust MCP-01 / P4 5.6–5.7；无 Java 对等对象。
pub struct ServerConfig {
    pub limits: HttpLimits,
    pub security: Security,
    pub legacy_sse: bool,
}

impl ServerConfig {
    /// 参数：limits 为资源预算，security 为安全上下文；返回：默认关闭旧 SSE 的现代传输配置。
    pub fn modern(limits: HttpLimits, security: Security) -> Self {
        Self {
            limits,
            security,
            legacy_sse: false,
        }
    }

    /// 参数：self 为现有配置；返回：启用旧 SSE、其余预算与安全策略保持的配置。
    pub fn with_legacy(mut self) -> Self {
        self.legacy_sse = true;
        self
    }
}
