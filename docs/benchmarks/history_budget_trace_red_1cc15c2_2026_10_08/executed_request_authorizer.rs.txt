//! 历史矩阵授权回调的定长诊断；不替代真实策略或修改请求预算。
use diskgraph_core::{Authorizer, Decision, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

/// 委托真实策略并记录最多八次回调耗时的测试对象；无 Java 对等实现。
pub(super) struct RequestAuthorizer {
    policy: PolicyAuthorizer,
    started: Instant,
    calls: Cell<usize>,
    timings: RefCell<[(Duration, Duration); 8]>,
}
impl RequestAuthorizer {
    /// 参数：原真实策略；返回：不改变授权决定、版本与到期时间的定长诊断对象。
    pub(super) fn new(policy: PolicyAuthorizer) -> Self {
        Self {
            policy,
            started: Instant::now(),
            calls: Cell::new(0),
            timings: RefCell::new([(Duration::ZERO, Duration::ZERO); 8]),
        }
    }
    /// 参数：无；返回：无，仅失败后输出计数及相对耗时，不输出身份、路径或正文。
    pub(super) fn report(&self) {
        let calls = self.calls.get();
        eprintln!(
            "HISTORY_AUTHORITY_DIAGNOSTIC calls={calls} wall_ms={} recorded_offsets_and_costs={:?}",
            self.started.elapsed().as_secs_f64() * 1000.0,
            &self.timings.borrow()[..calls.min(8)]
        );
    }
}
impl Authorizer for RequestAuthorizer {
    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        let started = Instant::now();
        let decision = self.policy.decide(principal, permission, scope);
        let index = self.calls.get();
        if index < 8 {
            self.timings.borrow_mut()[index] =
                (started.duration_since(self.started), started.elapsed());
        }
        self.calls.set(index.saturating_add(1));
        decision
    }
    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }
    fn expires_at_unix_seconds(&self) -> Option<u64> {
        self.policy.expires_at_unix_seconds()
    }
}
