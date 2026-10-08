//! 内容请求的连续撤权依赖；来源：DiskGraph CT-01/02。
use super::InspectionRequest;
use crate::{Engine, EngineError, request_withdrawal_witness::RequestWithdrawalWitness};
use diskgraph_core::{BusinessError, Permission};
use diskgraph_store::ControlStore;
use std::time::Instant;

/// 原请求内容权限的负向见证及未知平台的保守授权代次。
/// 来源：DiskGraph 原生 CT-01/02 内容请求生命周期，没有 Java 对照对象。
pub(super) struct ContentWithdrawal {
    witness: RequestWithdrawalWitness,
    generation: u64,
}
impl ContentWithdrawal {
    /// 参数：engine/request 为原控制连接与请求身份，deadline 为原期限；返回：非授权凭据。
    pub(super) fn capture(
        engine: &Engine,
        request: &InspectionRequest<'_>,
        deadline: Instant,
    ) -> Result<Self, EngineError> {
        let control = engine.control_until(deadline)?;
        control
            .with_read_deadline(deadline, |control| {
                Ok(Self {
                    witness: RequestWithdrawalWitness::capture_permission(
                        control,
                        request.principal,
                        request.scope_id,
                        &Permission::ContentRead,
                    )?,
                    generation: control.authorization_generation()?,
                })
            })
            .map_err(|error| match error {
                EngineError::Store(error)
                    if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                        || error.is_busy()
                        || error.is_interrupted() =>
                {
                    BusinessError::BudgetExceeded.into()
                }
                other => other,
            })
    }
    /// 参数：control 为原连接且已复验实时授权；返回：已撤权拒绝，未知代次变化冲突。
    pub(super) fn check_after_live_authorization(
        &self,
        control: &ControlStore,
    ) -> Result<(), EngineError> {
        self.witness.check(control)?;
        if !self.witness.has_native_watch()
            && control.authorization_generation()? != self.generation
        {
            return Err(BusinessError::Conflict.into());
        }
        Ok(())
    }
}
