use diskgraph_core::{Authorizer, Decision, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
use std::cell::{Cell, RefCell};

/// 在指定真实授权调用中同步外部控制连接；来源：D24 终态撤权验收。
pub(crate) struct CallbackAuthorizer {
    pub(crate) calls: Cell<usize>,
    policy: PolicyAuthorizer,
    callback: RefCell<Box<dyn FnMut(usize)>>,
}

impl CallbackAuthorizer {
    /// 参数：policy 为真实能力上限，callback 在实际授权调用时执行；返回：保留原决定的测试授权器。
    pub(crate) fn new(policy: PolicyAuthorizer, callback: impl FnMut(usize) + 'static) -> Self {
        Self {
            calls: Cell::new(0),
            policy,
            callback: RefCell::new(Box::new(callback)),
        }
    }
}

impl Authorizer for CallbackAuthorizer {
    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        (self.callback.borrow_mut())(call);
        self.policy.decide(principal, permission, scope)
    }

    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }
}
