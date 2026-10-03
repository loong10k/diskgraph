//! plan_request：既有文件操作职责的原生 Rust 实现。
use diskgraph_core::FileActionKind;
use diskgraph_core::PrincipalId;
use diskgraph_core::ScopeId;
use diskgraph_store::RecoveryRule;
use std::path::Path;

/// 聚合一次计划构建的主体、范围、动作、节点和恢复参数。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::PlanRequest`，保留既有语义。
/// Everything one plan build needs, kept together so the builder's own
/// signature stays readable.
pub(super) struct PlanRequest<'a> {
    pub(super) scope_id: &'a ScopeId,
    pub(super) principal: &'a PrincipalId,
    pub(super) action: FileActionKind,
    pub(super) node_ids: &'a [u64],
    pub(super) target: Option<&'a Path>,
    pub(super) max_bytes: u64,
    pub(super) recovery: RecoveryRule,
}
