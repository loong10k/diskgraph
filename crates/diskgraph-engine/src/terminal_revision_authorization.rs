//! revision 读取后终检；来源：OpenSpec SC-04，授权观察不续期数据读取。
use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, ScopeId};
use diskgraph_store::{ControlStore, SqliteSnapshotStore, StoreError};
use std::time::Instant;

impl Engine {
    /// 在能力回调/编码之后读取新鲜过滤归属，不复用消费者连接。
    /// 参数：revision/scope 为首次授权身份，control 是当前终检 guard，deadline 是固定授权期限。
    /// 返回：隔离/未绑定/不匹配为拒权，无法完成观察为原错误或预算失败。
    /// 独立 WAL 只读连接不获取共享 graph Mutex，不造成 control→graph 的锁反转。
    pub(super) fn require_terminal_revision_ownership(
        &self,
        revision: &str,
        scope: &ScopeId,
        control: &ControlStore,
        deadline: Instant,
    ) -> Result<(), EngineError> {
        let result = (|| {
            let server =
                control.with_read_deadline(deadline, |control| control.existing_server_id())?;
            let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
            if !reader.revision_ownership_matches(revision, server.as_str(), scope.as_str())? {
                return Err(BusinessError::PermissionDenied.into());
            }
            if Instant::now() >= deadline {
                return Err(BusinessError::BudgetExceeded.into());
            }
            Ok(())
        })();
        match result {
            Err(EngineError::Store(error))
                if matches!(error, StoreError::BudgetExceeded)
                    || error.is_busy()
                    || error.is_interrupted() =>
            {
                Err(BusinessError::BudgetExceeded.into())
            }
            other => other,
        }
    }
}
