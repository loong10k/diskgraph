//! approval_issuer：既有文件操作职责的原生 Rust 实现。
use crate::ops_error::OpsError;
use crate::ops_time::now_ms;
use crate::plan_digest::plan_digest;
use diskgraph_store::Approval;
use diskgraph_store::ControlStore;
use diskgraph_store::Plan;

/// 在既有控制库上登记可信宿主签发和撤销的计划批准。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::ApprovalIssuer`，保留既有语义。
/// Mints and verifies approvals. Only a trusted surface calls `issue`; the
/// agent-facing path can only `verify`.
pub struct ApprovalIssuer<'a> {
    control: &'a mut ControlStore,
}

impl<'a> ApprovalIssuer<'a> {
    /// 创建原有状态对象。
    /// 参数：control 为已持有的可变控制库引用，批准方由可信调用者提供。
    /// 返回：持有相同依赖的新对象。
    pub fn new(control: &'a mut ControlStore) -> Self {
        Self { control }
    }

    /// 登记可信批准并绑定计划摘要。
    /// 参数：plan 为完整计划；issued_by 为可信批准方名称；ttl_ms 为有效毫秒数。
    /// 返回：已登记的 Approval 记录或控制库错误。
    /// Issues an approval bound to the plan's current digest.
    pub fn issue(
        &mut self,
        plan: &Plan,
        issued_by: &str,
        ttl_ms: u64,
    ) -> Result<Approval, OpsError> {
        let digest = plan_digest(plan);
        let approval = Approval {
            approval_ref: format!("ap-{}", uuid::Uuid::new_v4()),
            plan_id: plan.plan_id.clone(),
            plan_digest: digest,
            principal: plan.principal.clone(),
            action: plan.action,
            issued_by: issued_by.to_owned(),
            issued_at_unix_ms: now_ms(),
            expires_at_unix_ms: now_ms() + ttl_ms,
            revoked: false,
        };
        self.control.insert_approval(&approval)?;
        Ok(approval)
    }

    /// 撤销指定批准。
    /// 参数：approval_ref 指定已登记的批准。
    /// 返回：撤销写入成功或控制库错误。
    /// Revokes an issued approval before it is used.
    pub fn revoke(&mut self, approval_ref: &str) -> Result<(), OpsError> {
        self.control.revoke_approval(approval_ref)?;
        Ok(())
    }
}
