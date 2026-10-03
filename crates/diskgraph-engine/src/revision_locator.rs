//! 基于 revision 实际归属的原始节点定位窄读，不接受显示路径作为目标。

use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, QueryBudget, QueryReadBudget};
use diskgraph_store::{StoreError, StoredNodeLocator};

impl Engine {
    /// 授权读取已发布节点的明确编码定位与自身修改时间。
    /// 参数：revision_id/node_id 为固定对象，身份与授权器为请求能力上限；budget 为 1–1000 ms 的读取额度。
    /// 返回：当前平台可解析的无损定位，或节点不存在、旧数据不可用、预算/授权失败。
    /// 此接口只读取观测元数据；实际文件访问仍须实时身份及原生根句柄验证。
    pub fn revision_node_locator(
        &self,
        revision_id: &str,
        node_id: u64,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        budget: QueryBudget,
    ) -> Result<StoredNodeLocator, EngineError> {
        let budget = budget.validated()?;
        self.with_authorized_revision_reader(
            revision_id,
            principal,
            authorizer,
            budget.deadline_ms,
            |reader, snapshot, deadline| {
                let mut reads = QueryReadBudget::new(budget, deadline)?;
                let stored = reader
                    .native_locator_bounded(snapshot, node_id, &mut reads)
                    .map_err(|error| match error {
                        StoreError::UnsupportedLocator(_) => {
                            EngineError::Business(BusinessError::Unsupported)
                        }
                        other => EngineError::Store(other),
                    })?
                    .ok_or(BusinessError::NotFound)?;
                if stored.locator.is_none() {
                    return Err(BusinessError::Unsupported.into());
                }
                #[cfg(test)]
                crate::revision_locator_tests::after_read(deadline);
                Ok(stored)
            },
        )
    }
}
