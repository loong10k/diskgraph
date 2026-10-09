use crate::control_database_identity::ControlDatabaseIdentity;
use rusqlite::Connection;
use std::sync::atomic::{AtomicU64, Ordering};

/// 仅由一个 ControlStore 强持的连接代次；请求与 registry 只保留弱引用。
/// 来源：原生 Rust Arc/Weak 生命周期；不是第二个数据库连接或权限 owner。
pub(crate) struct ControlStoreIncarnation {
    identity: Option<ControlDatabaseIdentity>,
    // 仅原事务已经确认并提交的授权代次下界，不表示当前授权或外部连接未变化。
    committed_generation: AtomicU64,
}

impl ControlStoreIncarnation {
    /// 为实际打开的控制连接建立一个不可复用的内存代次。
    /// 参数：connection 为已经校验完成的连接；返回：只含可靠原生身份或未知能力的代次。
    pub(crate) fn new(connection: &Connection) -> Self {
        Self {
            identity: ControlDatabaseIdentity::capture(connection),
            committed_generation: AtomicU64::new(0),
        }
    }

    /// 读取原连接打开时捕获的完整身份，不重新访问文件系统。
    /// 参数：无；返回：固定小身份的复制，未知能力保持 None。
    pub(crate) fn identity(&self) -> Option<ControlDatabaseIdentity> {
        self.identity
    }

    /// 记录原事务提交成功的代次。参数：generation为原事务读取的计数；返回：无，只保留单调下界。
    pub(crate) fn record_committed_generation(&self, generation: u64) {
        self.committed_generation
            .fetch_max(generation, Ordering::Release);
    }

    /// 参数：无；返回：本代次已知提交的下界，零不证明未变化，也不能授予权限。
    pub(crate) fn committed_generation(&self) -> u64 {
        self.committed_generation.load(Ordering::Acquire)
    }
}
