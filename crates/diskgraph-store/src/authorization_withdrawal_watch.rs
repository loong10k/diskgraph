use crate::withdrawal_entry::WithdrawalEntry;
use crate::{AuthorizationWithdrawalStatus, ControlStore};
use std::sync::{Arc, atomic::Ordering};

/// 由请求持有的负向见证，不能授予权限，也不延长连接或控制库生命周期。
/// 来源：DiskGraph 原生 Rust D45 的单调请求级撤权订阅。
pub struct AuthorizationWithdrawalWatch {
    pub(crate) entry: Arc<WithdrawalEntry>,
}

impl AuthorizationWithdrawalWatch {
    /// 纯内存检查本请求对应的原连接与已提交撤权事实。
    /// 参数：store 必须是本请求原 ControlStore；返回：撤权、未知或 typed 代次失效，未知不等于获准。
    pub fn status_for(&self, store: &ControlStore) -> AuthorizationWithdrawalStatus {
        // Weak 持有旧分配地址，原代次内存不会在本请求存活时被另一 Arc 复用。
        if !std::ptr::eq(
            self.entry.incarnation.as_ptr(),
            Arc::as_ptr(&store.withdrawal_incarnation),
        ) || !self.entry.is_live()
        {
            return AuthorizationWithdrawalStatus::Invalidated;
        }
        if self.entry.withdrawn.load(Ordering::Acquire) {
            AuthorizationWithdrawalStatus::Withdrawn
        } else {
            AuthorizationWithdrawalStatus::Unchanged
        }
    }
}
