use crate::control_database_identity::ControlDatabaseIdentity;
use crate::control_store_incarnation::ControlStoreIncarnation;
use diskgraph_core::{Permission, PrincipalId, ScopeId};
use std::sync::{
    Weak,
    atomic::{AtomicBool, Ordering},
};

/// 单个请求的固定依赖与单调撤权标记；registry 对本对象也只保存 Weak。
/// 来源：原生 Rust D45 成功提交后的负向通知，不是 Allow 缓存。
pub(crate) struct WithdrawalEntry {
    pub(crate) identity: ControlDatabaseIdentity,
    pub(crate) incarnation: Weak<ControlStoreIncarnation>,
    pub(crate) principal: PrincipalId,
    pub(crate) scope: ScopeId,
    pub(crate) permission: Permission,
    pub(crate) policy_epoch: Option<u64>,
    pub(crate) observed_generation: u64,
    pub(crate) withdrawn: AtomicBool,
}

impl WithdrawalEntry {
    /// 判断原连接是否仍存活，不升级为强持连接代次或接触 OS/SQL。
    /// 参数：无；返回：仍由原 ControlStore 强持时为 true。
    pub(crate) fn is_live(&self) -> bool {
        self.incarnation.strong_count() != 0
    }

    /// 发布已经完成持久提交的适用撤权；标记只从 false 单向变化为 true。
    /// 参数：无；返回：无，不调用用户代码或数据库。
    pub(crate) fn withdraw(&self) {
        self.withdrawn.store(true, Ordering::Release);
    }
}
