use super::ProbeLimits;
use super::probe_budget::ProbeBudget;
use super::probe_failure::ProbeFailure;
use crate::{ProbeHost, ProbeRecovery};
use std::ops::{Deref, DerefMut};

/// 原生探针测试的原预算与外部恢复责任；来源：Rust PF-06，无 Java 对等对象。
/// 仅测试使用；宿主绑定不重建期限，预算结束后实际排空原资源池。
pub(super) struct NativeProbeTestBudget {
    budget: Option<ProbeBudget>,
    recovery: ProbeRecovery,
}

impl NativeProbeTestBudget {
    /// 参数：limits 为原测试额度；返回：同一预算及原宿主恢复责任，错误不降级。
    pub(super) fn new(limits: &ProbeLimits) -> Result<Self, ProbeFailure> {
        let mut budget = ProbeBudget::new(limits)?;
        let (host, recovery) =
            ProbeHost::new(1).map_err(|error| ProbeFailure::Io(error.to_string()))?;
        match budget.check() {
            Ok(()) => budget.bind_probe_host(host.registry)?,
            // 原用例在出生前取消/到期；保持构造成功，由实际执行边界返回原失败。
            Err(ProbeFailure::Cancelled | ProbeFailure::Deadline) => {}
            Err(error) => return Err(error),
        }
        Ok(Self {
            budget: Some(budget),
            recovery,
        })
    }
}

impl Deref for NativeProbeTestBudget {
    type Target = ProbeBudget;

    fn deref(&self) -> &Self::Target {
        self.budget.as_ref().expect("original test budget is live")
    }
}

impl DerefMut for NativeProbeTestBudget {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.budget.as_mut().expect("original test budget is live")
    }
}

impl Drop for NativeProbeTestBudget {
    fn drop(&mut self) {
        // 先结束同一会话，恢复句柄仍在业务预算及出生 catch 外存活。
        drop(self.budget.take());
        let mut reported = false;
        loop {
            match self.recovery.drain() {
                Ok(true) => {
                    assert_eq!(
                        self.recovery
                            .occupied_slots()
                            .expect("original recovery capacity"),
                        0
                    );
                    break;
                }
                Ok(false) => {}
                Err(error) if !reported => {
                    eprintln!("native probe test recovery retains original owner: {error}");
                    reported = true;
                }
                Err(_) => {}
            }
            // 测试宿主的兼容恢复等待不代表产品已经实现有限退出。
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
