//! 历史增长与变化的实际归属资格；来源：DiskGraph 原生 Rust D34 / Q-04。

use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, QueryBudget, QueryReadBudget, ScopeId, query_deadline};
use diskgraph_store::SqliteSnapshotStore;

impl Engine {
    /// 核对两个已授权 revision 是否属于同一本机持久命名空间。
    /// 参数：left_reader/right_reader 为原请求已有的固定读取连接，revision 为原已解析历史标识。
    /// 返回：双方本机 scope 均有效且相同为 true，不同为 false；未绑定、异服务器、撤销或读取失败仍报错。
    /// 此可信辅助方法不授予权限；调用方必须保留双侧首检、预算及终态授权，不得将 false 提前返回给客户端。
    pub fn history_revisions_share_namespace(
        &self,
        left_reader: &SqliteSnapshotStore,
        left_revision: &str,
        right_reader: &SqliteSnapshotStore,
        right_revision: &str,
    ) -> Result<bool, EngineError> {
        let budget = QueryBudget::default();
        let mut reads = QueryReadBudget::new(budget, query_deadline(budget)?)?;
        self.history_revisions_share_namespace_with_budget(
            left_reader,
            left_revision,
            right_reader,
            right_revision,
            &mut reads,
        )
    }

    /// 可信历史入口的归属资格也计入原共享账本；来源：原生 Rust Q-08 / D41。
    /// 参数：双方 reader/revision 是固定历史，reads 为原请求准备账本。
    /// 返回：有效本机 scope 相同为 true；未绑定、撤销、预算与读取错误明确返回。
    pub(super) fn history_revisions_share_namespace_with_budget(
        &self,
        left_reader: &SqliteSnapshotStore,
        left_revision: &str,
        right_reader: &SqliteSnapshotStore,
        right_revision: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<bool, EngineError> {
        let (left_server, left_scope) = left_reader
            .revision_ownership_with_budget(left_revision, reads)?
            .ok_or(BusinessError::PermissionDenied)?;
        let (right_server, right_scope) = right_reader
            .revision_ownership_with_budget(right_revision, reads)?
            .ok_or(BusinessError::PermissionDenied)?;
        let server = self.server_id()?;
        if left_server != server.as_str() || right_server != server.as_str() {
            return Err(BusinessError::PermissionDenied.into());
        }
        let left_scope = ScopeId::new(left_scope).map_err(|_| BusinessError::PermissionDenied)?;
        let right_scope = ScopeId::new(right_scope).map_err(|_| BusinessError::PermissionDenied)?;
        let control = self.control_store()?;
        let left = control.scope(&left_scope)?;
        let right = control.scope(&right_scope)?;
        if left.revoked || right.revoked {
            return Err(BusinessError::PermissionDenied.into());
        }
        // 注册 scope 的根定位不可变：同服务器的同 ScopeId 已绑定同一份无损根。
        // 不把显示路径、Base64 字符串表面相等或当前文件系统状态用作历史归属。
        Ok(left_scope == right_scope)
    }
}
