use crate::protocol::ToolProfile;
use diskgraph_core::PrincipalId;
use std::path::PathBuf;

/// stdio 固定绑定的本地主体；仍执行范围和策略校验，工具调用不能扩大授权。
/// 来源：原生 Rust MCP 本地传输契约。
pub const STDIO_PRINCIPAL: &str = "local-user";

/// MCP 服务配置；来源：原生 Rust diskgraph-mcp::McpConfig。
#[derive(Clone, Debug)]
pub struct McpConfig {
    pub data_dir: PathBuf,
    pub profile: ToolProfile,
    pub principal: PrincipalId,
    /// 是否启用旧 HTTP+SSE 适配器，默认关闭。
    pub legacy_sse: bool,
}

impl Default for McpConfig {
    /// 构造原有本地服务默认配置。参数：无。返回：read-full、本地主体、关闭旧 SSE 的配置。
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("diskgraph-data"),
            profile: ToolProfile::ReadFull,
            principal: PrincipalId::new(STDIO_PRINCIPAL).expect("constant principal is valid"),
            legacy_sse: false,
        }
    }
}
