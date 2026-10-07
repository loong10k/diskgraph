use super::clock_sample::ClockSample;
use crate::EngineError;
use diskgraph_core::BusinessError;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
/// 原绝对计数期限；来源：PF-06 仅同出生本机私有 IPC，无 Java 对等对象。
/// 不认证 peer，不跨机器/重启；禁止作为远程 token 或未认证请求的预算来源。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockStamp {
    version: u16,
    domain: [u64; 3],
    expires_nanos: u64,
}
impl ClockStamp {
    /// 参数：original 为原调用绝对期限；返回：已扣耗时的有界时钟材料，过期拒绝。
    pub fn capture(original: Instant) -> Result<Self, EngineError> {
        // 必须先采原生时钟、后采剩余 Instant，采样间隙只能减少原预算。
        let clock = ClockSample::read()?;
        let remaining = original
            .checked_duration_since(Instant::now())
            .ok_or(BusinessError::BudgetExceeded)?;
        let nanos =
            u64::try_from(remaining.as_nanos()).map_err(|_| BusinessError::BudgetExceeded)?;
        let nanos = nanos
            .checked_sub(clock.margin_nanos)
            .filter(|n| *n > 0)
            .ok_or(BusinessError::BudgetExceeded)?;
        Ok(Self {
            version: 1,
            domain: clock.domain,
            expires_nanos: clock
                .nanos
                .checked_add(nanos)
                .ok_or(BusinessError::BudgetExceeded)?,
        })
    }
    /// 参数：maximum 为接收端已确认的策略上限；返回：原剩余期限，不刷新预算。
    pub fn adopt(&self, maximum: Duration) -> Result<Instant, EngineError> {
        let before = Instant::now();
        let clock = ClockSample::read()?;
        if self.version != 1 || self.domain != clock.domain {
            return Err(BusinessError::Conflict.into());
        }
        // 从同一原生绝对截止值扣除传输/排队成本，绝不重新采用发送时的 Duration。
        let remaining = self
            .expires_nanos
            .checked_sub(clock.nanos)
            .and_then(|value| value.checked_sub(clock.margin_nanos))
            .filter(|value| *value > 0)
            .ok_or(BusinessError::BudgetExceeded)?;
        if remaining == 0 || u128::from(remaining) > maximum.as_nanos() {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let deadline = before
            .checked_add(Duration::from_nanos(remaining))
            .ok_or(BusinessError::BudgetExceeded)?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(deadline)
    }
}
