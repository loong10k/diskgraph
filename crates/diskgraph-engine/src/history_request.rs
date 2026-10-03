use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, PrincipalId};
use diskgraph_store::SqliteSnapshotStore;
use std::time::Instant;

impl Engine {
    /// 双侧历史读取共用原始期限，编码之后检查实际双侧 scope/grant。
    /// 参数：left/right 为固定 revision，身份与 deadline 为同请求，consumer/finish 不得读取其他快照。
    /// 返回：预算内结果；任何错误或期限 partial 都必须先通过真实末段授权。
    #[allow(clippy::too_many_arguments)] // 双侧上下文和共享消费者/编码阶段均为独立必需输入。
    pub(super) fn with_history_readers_until<T>(
        &self,
        left: &str,
        right: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
        consumer: impl FnOnce(
            &SqliteSnapshotStore,
            &str,
            &SqliteSnapshotStore,
            &str,
        ) -> Result<T, EngineError>,
        mut finish: impl FnMut(&mut T, bool) -> Result<(), EngineError>,
    ) -> Result<T, EngineError> {
        let left_reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let right_reader =
            SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let left_scope =
            self.authorize_revision_with_reader(&left_reader, None, left, principal, authorizer)?;
        let right_scope =
            self.authorize_revision_with_reader(&right_reader, None, right, principal, authorizer)?;
        let left_snapshot = left_reader.revision(left)?.snapshot_id;
        let right_snapshot = right_reader.revision(right)?.snapshot_id;
        let result = consumer(&left_reader, &left_snapshot, &right_reader, &right_snapshot);
        let control = self.control_store()?;
        let scopes = [&left_scope, &right_scope];
        Self::require_terminal_relations(&control, authorizer, principal, &scopes)?;
        let mut result = result?;
        let expired = Instant::now() >= deadline;
        let encoded = finish(&mut result, expired);
        // guard 只限制本 Engine 重入；独立连接依然能撤权，编码后重新读取。
        Self::require_terminal_relations(&control, authorizer, principal, &scopes)?;
        encoded?;
        if !expired && Instant::now() >= deadline {
            let encoded = finish(&mut result, true);
            Self::require_terminal_relations(&control, authorizer, principal, &scopes)?;
            encoded?;
        }
        Ok(result)
    }
}
