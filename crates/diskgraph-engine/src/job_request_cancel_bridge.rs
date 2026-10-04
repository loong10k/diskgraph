//! 请求信号只约束成功认领的执行代次，不构成第二份任务状态。

use crate::{Engine, EngineError};
use diskgraph_core::BusinessError;
use diskgraph_store::{ControlStore, JobRecord, StoreError};
use std::sync::atomic::{AtomicBool, Ordering};

/// 将可信宿主的取消/明确拒权信号绑定到本执行器成功认领的固定 owner/fence。
/// 来源：DiskGraph 原生 Rust PF-06；不从 job_status 构造他人的停止资格。
pub(super) struct JobRequestCancelBridge<'a> {
    engine: &'a Engine,
    claimed: &'a JobRecord,
    request_cancel: Option<&'a AtomicBool>,
    deny_stop: Option<&'a AtomicBool>,
    local_stop: &'a AtomicBool,
    denial_observed: AtomicBool,
}

impl<'a> JobRequestCancelBridge<'a> {
    /// 参数：engine/claimed 为真实成功认领，signals 为请求局部两信号，local_stop 为本代执行 Arc。
    /// 返回：借用已有状态的桥，不持有连接、线程或新任务 owner。
    pub(super) fn new(
        engine: &'a Engine,
        claimed: &'a JobRecord,
        signals: Option<(&'a AtomicBool, &'a AtomicBool)>,
        local_stop: &'a AtomicBool,
    ) -> Self {
        Self {
            engine,
            claimed,
            request_cancel: signals.map(|(request, _)| request),
            deny_stop: signals.map(|(_, denied)| denied),
            local_stop,
            denial_observed: AtomicBool::new(false),
        }
    }

    fn pending(&self) -> bool {
        self.request_cancel
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
            || self
                .deny_stop
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
    }

    /// 在 claim 后、本机 Arc 登记前后消费原信号；无信号的旧入口不额外读取控制库。
    /// 参数：无；返回：继续准备，或本代请求停止/原真实权限及 owner 错误。
    pub(super) fn check(&self) -> Result<(), EngineError> {
        if !self.pending() {
            return Ok(());
        }
        let mut control = self.engine.control()?;
        // 先保留真实 owner/权限/格式失败；停止资格不能赋予工作或洗掉原授权错误。
        control.with_job_fence(
            &self.claimed.job_id,
            &self.claimed.owner,
            self.claimed.fencing_token,
            || Ok(()),
        )?;
        self.check_with_control(&mut control)
    }

    /// 消费 keeper 已通过真实 fence 的同一代次信号。
    /// 参数：control 为当前已持有 guard；返回：继续或协作停止；不重入控制锁或查询取消表。
    pub(super) fn check_with_control(&self, control: &mut ControlStore) -> Result<(), EngineError> {
        // 明确拒权优先且不写 cancel_requested；内部 keeper 停止 Atomic 不能推导此原因。
        if self
            .deny_stop
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            self.denial_observed.store(true, Ordering::SeqCst);
            self.local_stop.store(true, Ordering::SeqCst);
            return Err(BusinessError::PermissionDenied.into());
        }
        if self
            .request_cancel
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            if !control.request_cancel_generation(
                &self.claimed.job_id,
                &self.claimed.owner,
                self.claimed.fencing_token,
            )? {
                return Err(StoreError::StaleOwner.into());
            }
            // 只通知借用的本代 Atomic；条件更新之后也不按 job_id 查找或改变新代次 Arc。
            self.local_stop.store(true, Ordering::SeqCst);
            return Err(BusinessError::Conflict.into());
        }
        Ok(())
    }

    /// 参数：无；返回：桥实际消费过明确拒权，而非根据内部执行停止位猜测。
    pub(super) fn denial_observed(&self) -> bool {
        self.denial_observed.load(Ordering::SeqCst)
    }

    /// 仅把本代明确拒权与来源明确的纯提交停止回传为拒权；其他真实错误保持原值。
    /// 参数：outcome 为已结束执行的原结果；返回：成功事实不变，存储/owner/格式错误不归一。
    /// 必须在已提交 receipt 事实保护之后调用，不能覆写已经提交的结果。
    pub(super) fn preserve_outcome(
        &self,
        outcome: Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        if !self.denial_observed() {
            return outcome;
        }
        match outcome {
            Err(EngineError::Store(StoreError::Conflict(message)))
                if message == "scan cancelled before commit" =>
            {
                // 此固定错误只由本机纯 commit 取消门禁产生，不匹配任何原始工具错误文本。
                Err(BusinessError::PermissionDenied.into())
            }
            result => result,
        }
    }
}
