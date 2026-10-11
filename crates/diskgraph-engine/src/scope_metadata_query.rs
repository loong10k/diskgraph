//! 注册范围元数据对外读取：沿原请求期限授权，编码后重新观察实时权限。
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, ScopeId};
use diskgraph_store::ScopeRecord;
use std::time::Instant;

impl Engine {
    /// 参数：scope为目标范围，principal/authorizer为实际请求身份，deadline为原期限，consumer仅编码缓冲结果。
    /// 返回：初始及末段授权通过后的消费结果；撤权、到期或超预算时不交付已编码数据。
    /// 保留scope/scope_until可信内部接口，公开CLI不得以原始记录读取替代本授权入口。
    pub fn with_scope_metadata_until<T>(
        &self,
        scope: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
        consumer: impl FnOnce(&ScopeRecord) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let expiry = authorizer.expires_at_unix_seconds();
        self.require_reader_capability_until(authorizer, principal, scope, expiry, deadline)?;
        let record = self.scope_until(scope, deadline)?;
        // 消费者只生成未公开缓冲，不持控制锁；失败结果同样不能绕过末段撤权。
        let result = consumer(&record);
        let timely = self.observe_terminal_relation(authorizer, principal, scope, expiry)?;
        if !timely || Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        result
    }
}
