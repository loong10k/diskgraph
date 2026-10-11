use crate::recovery_slot::ActiveSlot;
use crate::{
    Engine, ExecutorParts, ExecutorResourceOwner, SupervisorParts, SupervisorRecoveryError,
};
use diskgraph_core::BusinessError;
use std::sync::Arc;
use std::time::Instant;
/// 原 Engine、恢复责任和 ACTIVE 容量的监督持有者；来源：PF-06，无 Java 对等对象。
/// 必须在前台 catch_unwind 外保留；Drop 不表示回收，不建立进程或 IPC 执行信任。
#[must_use = "retain the original owner until explicit retirement is confirmed"]
pub struct SupervisorOwner {
    resources: ExecutorResourceOwner,
    slot: Option<ActiveSlot>,
}
impl SupervisorOwner {
    /// 参数：parts 为全部原材料，deadline 为原绑定期限；返回：同一原绑定或原材料整体拒绝，不丢弃恢复责任。
    pub fn bind(mut parts: SupervisorParts, deadline: Instant) -> Result<Self, SupervisorParts> {
        if parts.slot.verify_active(deadline).is_err() {
            return Err(parts);
        }
        let SupervisorParts {
            engine,
            scan,
            slot,
            #[cfg(windows)]
            probe,
        } = parts;
        let resources = match ExecutorResourceOwner::bind(ExecutorParts {
            engine,
            scan,
            #[cfg(windows)]
            probe,
        }) {
            Ok(resources) => resources,
            Err(ExecutorParts {
                engine,
                scan,
                #[cfg(windows)]
                probe,
            }) => {
                return Err(SupervisorParts {
                    engine,
                    scan,
                    slot,
                    #[cfg(windows)]
                    probe,
                });
            }
        };
        Ok(Self {
            resources,
            slot: Some(slot),
        })
    }

    /// 参数：无；返回：尚未开始销毁的原 Engine 引用；退休后不再提供执行入口。
    pub fn engine(&self) -> Option<&Arc<Engine>> {
        self.resources.engine()
    }

    /// 参数：deadline 为原单次恢复绝对期限；返回：真实原资源确认及原槽释放才为 true。
    /// Pending/错误均保留原 owner；此方法不能代替真实前台结束、runner join 和进程退出验收。
    pub fn poll_retirement(&mut self, deadline: Instant) -> Result<bool, SupervisorRecoveryError> {
        if Instant::now() >= deadline {
            return Err(crate::EngineError::from(BusinessError::BudgetExceeded).into());
        }
        if self.slot.is_none() {
            return Ok(true);
        }
        if !self.resources.poll_retirement(deadline)? {
            return Ok(false);
        }
        self.slot
            .as_mut()
            .expect("original active slot retained")
            .confirm_original_cleanup(deadline)?;
        // 同一原锁的 CLEAN 同步回读及显式解锁成功后立即消费；失败不消费原能力。
        drop(self.slot.take());
        Ok(true)
    }
}
