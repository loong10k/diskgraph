use crate::ControlStore;
use crate::control_store_incarnation::ControlStoreIncarnation;
use std::sync::{Arc, Weak};

/// 绑定原控制连接的已提交授权代次下界，只能确认负向变化，不能缓存允许。
/// 来源：DiskGraph原生请求终检语义；不支持跨连接发现，缺少本地记录仍须实时SQL。
pub struct CommittedAuthorizationGeneration {
    incarnation: Weak<ControlStoreIncarnation>,
}

impl CommittedAuthorizationGeneration {
    /// 参数：store须为捕获时的原连接；返回：已知代次下界，原连接失效或替换返回None。
    /// 零或不超过原基线均不证明未变化；本方法不执行SQL，不保留连接/文件强所有权。
    pub fn latest_known_for(&self, store: &ControlStore) -> Option<u64> {
        if !std::ptr::eq(
            self.incarnation.as_ptr(),
            Arc::as_ptr(&store.withdrawal_incarnation),
        ) {
            return None;
        }
        Some(store.withdrawal_incarnation.committed_generation())
    }
}

impl ControlStore {
    /// 捕获原连接代次的负向提交见证。参数：无；返回：只含弱绑定、不执行SQL的见证。
    /// 调用方须另捕获原持久授权代次；本对象不授予权限、不代表可靠跨连接通知。
    pub fn committed_authorization_generation_witness(&self) -> CommittedAuthorizationGeneration {
        CommittedAuthorizationGeneration {
            incarnation: Arc::downgrade(&self.withdrawal_incarnation),
        }
    }
}
