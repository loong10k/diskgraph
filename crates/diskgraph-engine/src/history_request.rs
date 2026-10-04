use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, PrincipalId, QueryBudget, QueryReadBudget};
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
        budget: QueryBudget,
        consumer: impl FnOnce(
            &SqliteSnapshotStore,
            &str,
            &SqliteSnapshotStore,
            &str,
            &mut QueryReadBudget,
            bool,
        ) -> Result<T, EngineError>,
        mut finish: impl FnMut(&mut T, bool) -> Result<(), EngineError>,
    ) -> Result<T, EngineError> {
        let mut reads = QueryReadBudget::new(budget, deadline)?;
        let left_reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let right_reader =
            SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let left_scope = self.authorize_revision_with_budget(
            &left_reader,
            None,
            left,
            principal,
            authorizer,
            &mut reads,
        )?;
        let right_scope = self.authorize_revision_with_budget(
            &right_reader,
            None,
            right,
            principal,
            authorizer,
            &mut reads,
        )?;
        // 双侧都已按本服务器真实归属授权；同 ScopeId 绑定同一不可变注册根。
        // 目标准入错误与 consumer 错误一起经过原双侧末检，不提前返回错误或部分结果。
        let result = (|| {
            let left_snapshot = left_reader.revision_snapshot_with_budget(left, &mut reads)?;
            let right_snapshot = right_reader.revision_snapshot_with_budget(right, &mut reads)?;
            consumer(
                &left_reader,
                &left_snapshot,
                &right_reader,
                &right_snapshot,
                &mut reads,
                left_scope == right_scope,
            )
        })();
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
