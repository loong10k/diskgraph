//! 将引擎错误与产生错误的命令目录上下文关联。

use crate::EngineError;

/// 将引擎错误与产生错误的命令目录上下文关联。
/// 来源：原生 Rust diskgraph-engine::ContextualEngineError。
/// An engine error plus which catalog entry produced it, so MCP and the CLI
/// can name the family without parsing message text.
#[derive(Debug)]
pub struct ContextualEngineError {
    pub error: EngineError,
    pub context: String,
}

impl std::fmt::Display for ContextualEngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (while serving {})", self.error, self.context)
    }
}
impl std::error::Error for ContextualEngineError {}
