//! ops_error：既有文件操作职责的原生 Rust 实现。
use diskgraph_store::StoreError;

/// 文件操作的业务错误与底层错误转换，保持既有错误文本及类型。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::OpsError`，保留既有语义。
/// Errors surfaced by the ops layer. They mirror the business error codes so
/// the CLI and MCP can pass them through unchanged.
#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error("no such scope: {0}")]
    NoSuchScope(String),
    #[error("object not found in the bound revision: node {0}")]
    NoSuchNode(u64),
    #[error("the request resolved to an empty object set")]
    EmptyPlan,
    #[error("parent/child overlap could not be resolved for node {0}")]
    OverlappingObjects(u64),
    #[error("the action does not belong to this plan's action")]
    ActionMismatch,
    #[error("not authorized: {0}")]
    NotAuthorized(String),
    #[error("precondition failed: {0}")]
    Stale(String),
    #[error("conflicts with in-flight work: {0}")]
    Conflict(String),
    #[error("the target already exists and overwrite is not permitted")]
    TargetExists,
    #[error("a same-volume move is not possible across devices")]
    CrossVolume,
    #[error("irrecoverable: {0}")]
    Irrecoverable(String),
    /// Internal: a drill fault parked the operation for reconciliation. The
    /// apply loop turns this into a NeedsAttention state; production code
    /// never returns it.
    #[error("parked for attention: {0}")]
    ParkNeedsAttention(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Engine(#[from] diskgraph_engine::EngineError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
