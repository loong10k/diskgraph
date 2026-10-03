use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, QueryBudget, QueryReadBudget};
use diskgraph_store::StoredWindowsObservation;

impl Engine {
    /// 按 revision 实际归属授权读取完整 Windows 属性，不访问文件或转换外平台路径。
    /// 来源：DiskGraph 原生 Rust D31；无 Java 对应方法。
    /// 参数：revision_id/node_id 为固定对象，principal/authorizer 为请求能力上限，budget 为累计窄读预算。
    /// 返回：完整观测、明确缺失原因，或节点不存在、损坏、预算及首末实时授权失败。
    pub fn revision_windows_observation(
        &self,
        revision_id: &str,
        node_id: u64,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        budget: QueryBudget,
    ) -> Result<StoredWindowsObservation, EngineError> {
        let budget = budget.validated()?;
        self.with_authorized_revision_reader(
            revision_id,
            principal,
            authorizer,
            budget.deadline_ms,
            |reader, snapshot, deadline| {
                let mut reads = QueryReadBudget::new(budget, deadline)?;
                let value = reader
                    .windows_observation_bounded(snapshot, node_id, &mut reads)?
                    .ok_or(BusinessError::NotFound)?;
                #[cfg(test)]
                crate::revision_windows_observation_tests::after_read(deadline);
                Ok(value)
            },
        )
    }
}
