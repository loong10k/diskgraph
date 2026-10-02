//! 绑定真实主体、scope、精确文件、范围、字节/块预算及协作取消信号。

use diskgraph_core::{PrincipalId, ScopeId};
use std::path::Path;

/// 绑定真实主体、scope、精确文件、范围、字节/块预算及协作取消信号。
/// 来源：原生 Rust diskgraph-engine::content::InspectionRequest。
/// What one bounded inspection needs.
pub struct InspectionRequest<'a> {
    pub scope_id: &'a ScopeId,
    pub principal: &'a PrincipalId,
    /// The exact object, inside the scope's root.
    pub path: &'a Path,
    pub offset: u64,
    /// The whole byte budget; a read never loads more than this.
    pub max_bytes: u64,
    /// Set by cooperative cancellation between chunks.
    pub cancel: Option<&'a std::sync::atomic::AtomicBool>,
    /// Chunk size for reads and digests; bounds peak memory.
    pub chunk_bytes: usize,
}
