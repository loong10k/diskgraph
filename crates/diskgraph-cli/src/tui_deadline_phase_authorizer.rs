//! 仅用于失败阶段观测及初次回调延迟夹具；来源：OpenSpec Q-08，保持真实策略决定与版本不变。

use diskgraph_core::{Authorizer, Decision, Permission, PrincipalId, ScopeId};
use std::cell::Cell;
use std::time::{Duration, Instant};

/// 有界记录真实授权回调的次数和相对时间，不查询数据库或更改期限。
/// 来源：DiskGraph 原生 Rust TUI 验收夹具；无 Java 对应对象。
pub(super) struct TuiDeadlinePhaseAuthorizer<'a> {
    delegate: &'a dyn Authorizer,
    started: Instant,
    calls: Cell<usize>,
    pause_first: Cell<Option<Duration>>,
    first: Cell<Option<Duration>>,
    last: Cell<Option<Duration>>,
}

impl<'a> TuiDeadlinePhaseAuthorizer<'a> {
    /// 参数：delegate 为原授权器；返回：只记录计时的透明委托。
    pub(super) fn new(delegate: &'a dyn Authorizer) -> Self {
        Self {
            delegate,
            started: Instant::now(),
            calls: Cell::new(0),
            pause_first: Cell::new(None),
            first: Cell::new(None),
            last: Cell::new(None),
        }
    }

    /// 参数：delay 为首次真实决定前的受控延迟；返回：无，不更改委托的权限或版本。
    /// 用于固定原请求初次到期阶段，不能给后续读取重建期限。
    pub(super) fn pause_first_decision(&self, delay: Duration) {
        self.pause_first.set(Some(delay));
    }

    /// 参数：无；返回：真实回调次数与首末进入时点，不能推断未观测的 SQL 阶段。
    pub(super) fn phases(&self) -> (usize, Option<u128>, Option<u128>) {
        (
            self.calls.get(),
            self.first.get().map(|value| value.as_micros()),
            self.last.get().map(|value| value.as_micros()),
        )
    }
}

impl Authorizer for TuiDeadlinePhaseAuthorizer<'_> {
    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        if let Some(delay) = self.pause_first.take() {
            std::thread::sleep(delay);
        }
        let elapsed = self.started.elapsed();
        if self.first.get().is_none() {
            self.first.set(Some(elapsed));
        }
        self.last.set(Some(elapsed));
        self.calls.set(self.calls.get().saturating_add(1));
        self.delegate.decide(principal, permission, scope)
    }

    fn policy_version(&self) -> u64 {
        self.delegate.policy_version()
    }
}
