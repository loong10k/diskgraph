//! 多侧查询从初始准入前到编码完成的撤权见证；来源：DiskGraph 原生请求授权契约。
use crate::{EngineError, request_withdrawal_witness::RequestWithdrawalWitness};
use diskgraph_core::{BusinessError, PrincipalId, ScopeId};
use diskgraph_store::{CommittedAuthorizationGeneration, ControlStore};

/// 绑定原控制连接的元数据读取依赖及未知通知平台的保守代次基线。
/// 来源：DiskGraph 原生历史和关系查询生命周期，无 Java 对照对象。
pub(super) struct RequestMetadataWithdrawals {
    witnesses: Vec<RequestWithdrawalWitness>,
    generation: u64,
    committed_generation: CommittedAuthorizationGeneration,
}

impl RequestMetadataWithdrawals {
    /// 参数：control 为原控制连接，principal/scopes 是图中实际所有者；返回：不能授予权限的见证。
    pub(super) fn capture(
        control: &ControlStore,
        principal: &PrincipalId,
        scopes: &[&ScopeId],
    ) -> Result<Self, EngineError> {
        let witnesses = scopes
            .iter()
            .map(|scope| RequestWithdrawalWitness::capture(control, principal, scope))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            witnesses,
            generation: control.authorization_generation()?,
            committed_generation: control.committed_authorization_generation_witness(),
        })
    }

    /// 在末段预算失败后保留原连接已知的精确撤权事实，不追加 SQL。
    /// 参数：engine 为原请求引擎，result 为原末段结果；返回：已知撤权拒绝或原结果。
    /// 原锁繁忙、连接失效和未知能力均不猜测拒权；此方法绝不能将错误改成成功。
    pub(super) fn prioritize_terminal_budget<T>(
        &self,
        engine: &crate::Engine,
        result: Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        if matches!(
            &result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ) {
            // 仅负向内存观察，不等待锁、不更新读取期限或复用已过期的 SQL guard。
            if let Ok(Some(control)) = engine.try_control_store() {
                for witness in &self.witnesses {
                    if matches!(
                        witness.check(&control),
                        Err(EngineError::Business(BusinessError::PermissionDenied))
                    ) {
                        return Err(BusinessError::PermissionDenied.into());
                    }
                }
                // 未知通知平台仅使用原连接已确认的提交下界；不延长SQL窗口或补造未变。
                if self
                    .witnesses
                    .iter()
                    .any(|witness| !witness.has_native_watch())
                    && self
                        .committed_generation
                        .latest_known_for(&control)
                        .is_some_and(|generation| generation > self.generation)
                {
                    return Err(BusinessError::Conflict.into());
                }
            }
        }
        result
    }

    /// 参数：control 为原连接且调用方已在原 SQL 窗口内复验实时拒权；返回：允许继续观察或拒绝。
    /// 本方法不授予权限；未知通知能力的任何授权变更均保守冲突。
    pub(super) fn check_after_live_authorization(
        &self,
        control: &ControlStore,
    ) -> Result<(), EngineError> {
        for witness in &self.witnesses {
            witness.check(control)?;
        }
        if self
            .witnesses
            .iter()
            .any(|witness| !witness.has_native_watch())
            && control.authorization_generation()? != self.generation
        {
            return Err(BusinessError::Conflict.into());
        }
        Ok(())
    }
}
