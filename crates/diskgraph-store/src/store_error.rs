use thiserror::Error;

/// 保留 SQLite 扩展码和业务可识别失败的存储错误。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0} (extended_code={code:?})", code = .0.sqlite_extended_error_code())]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),
    #[error("revision not found: {0}")]
    RevisionNotFound(String),
    #[error("scope not found: {0}")]
    ScopeNotFound(String),
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("plan not found: {0}")]
    PlanNotFound(String),
    #[error("operation not found: {0}")]
    OperationNotFound(String),
    #[error("recovery entry not found: {0}")]
    RecoveryNotFound(String),
    #[error("approval required: {0}")]
    ApprovalRequired(String),
    #[error("the idempotency key was reused for a different request")]
    IdempotencyConflict,
    #[error("another owner holds this job or resource")]
    StaleOwner,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("retention violation: {0}")]
    RetentionViolation(String),
    #[error("invalid graph: {0}")]
    InvalidGraph(String),
    #[error("value is too large for SQLite INTEGER")]
    IntegerOverflow,
    #[error("query response budget exceeded by one record")]
    BudgetExceeded,
    /// 合法定位类型或编码不支持当前宿主的原生寻址。
    #[error("unsupported native locator: {0}")]
    UnsupportedLocator(String),
    #[error("unsupported SQLite schema version: {0}")]
    UnsupportedSchema(i64),
}

impl StoreError {
    /// SQLite 期限或取消中断；调用者可保留已读取结果并标记截断。
    /// 判断 SQLite 中断原因或操作终态。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：是否为期限/取消导致的 SQLite 中断。
    pub fn is_interrupted(&self) -> bool {
        matches!(self, Self::Sqlite(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::OperationInterrupted)
    }
}
