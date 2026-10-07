//! 共享 Engine 的 revision_authorization 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError, admin_scope};
use diskgraph_core::{
    Authorizer, BusinessError, Permission, PrincipalId, QueryBudget, QueryReadBudget, ScopeId,
};
use diskgraph_store::SqliteSnapshotStore;
use std::time::Duration;

impl Engine {
    /// 解析持久 server/scope 归属后求授权交集。
    /// 参数：expected_scope 仅为一致性断言，revision_id 与 principal/authorizer 为身份。
    /// 返回：实际 scope 或拒绝/查询失败。
    /// 按持久化归属解析 revision 并授权；客户端 scope 只能作为一致性断言。
    pub fn authorize_revision(
        &self,
        expected_scope: Option<&ScopeId>,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ScopeId, EngineError> {
        let reader = self.revision_reader()?;
        self.authorize_revision_with_reader(
            &reader,
            expected_scope,
            revision_id,
            principal,
            authorizer,
        )
    }
}

impl Engine {
    /// 初次 revision 解析与授权继承整次请求期限。
    /// 参数：expected_scope 为一致性断言，revision_id/主体/授权器为真实请求，deadline 由最外层生成。
    /// 返回：实际 scope 或拒权/存储失败；不重新建立 SQLite 执行期限。
    pub fn authorize_revision_until(
        &self,
        expected_scope: Option<&ScopeId>,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: std::time::Instant,
    ) -> Result<ScopeId, EngineError> {
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let ownership = reader.revision_ownership(revision_id)?;
        let scope = self.authorize_revision_owner_until(
            ownership,
            expected_scope,
            principal,
            authorizer,
            deadline,
        )?;
        // 短 SQL 的 VM hook 不保证触发；同步能力回调返回后也必须核对原绝对期限。
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(scope)
    }

    /// 复用当前 reader 解析真实归属后授权。
    /// 参数：reader、expected_scope、revision 及请求身份为上下文。
    /// 返回：真实 scope 或拒绝/存储失败。
    pub(super) fn authorize_revision_with_reader(
        &self,
        reader: &SqliteSnapshotStore,
        expected_scope: Option<&ScopeId>,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ScopeId, EngineError> {
        let ownership = reader.revision_ownership(revision_id)?;
        self.authorize_revision_owner(ownership, expected_scope, principal, authorizer)
    }

    /// 真实归属的原始字段与后续历史读取共用账本；来源：原生 Rust Q-08 / D41。
    /// 参数：reader/revision、可选范围断言及请求身份固定，reads 不得重置。
    /// 返回：实际授权 scope；预算、缺失归属与拒权保留原错误。
    pub(super) fn authorize_revision_with_budget(
        &self,
        reader: &SqliteSnapshotStore,
        expected_scope: Option<&ScopeId>,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        reads: &mut QueryReadBudget,
    ) -> Result<ScopeId, EngineError> {
        let ownership = reader.revision_ownership_with_budget(revision_id, reads)?;
        self.authorize_revision_owner_until(
            ownership,
            expected_scope,
            principal,
            authorizer,
            reads.deadline(),
        )
    }

    fn authorize_revision_owner(
        &self,
        ownership: Option<(String, String)>,
        expected_scope: Option<&ScopeId>,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ScopeId, EngineError> {
        let (server, scope) =
            ownership.ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
        let scope = ScopeId::new(scope)
            .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
        if server != self.server_id()?.as_str()
            || expected_scope.is_some_and(|expected| expected != &scope)
            || self.scope(&scope)?.revoked
        {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        self.require(authorizer, principal, &Permission::MetadataRead, &scope)?;
        Ok(scope)
    }
}

impl Engine {
    /// 通过 snapshot 对应 revision 的真实归属授权。
    /// 参数：snapshot_id、principal、authorizer 为对象与请求身份。
    /// 返回：授权成功或未绑定/撤销/权限拒绝。
    /// 旧 snapshot API 经 revision 的实际归属授权；未绑定历史明确要求重新索引。
    pub fn authorize_snapshot(
        &self,
        snapshot_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        let revision = self
            .revision_reader()?
            .revision_for_snapshot(snapshot_id)?
            .ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
        self.authorize_revision(None, &revision, principal, authorizer)?;
        Ok(())
    }
}

impl Engine {
    /// 初次 snapshot 到 revision 归属解析沿用同一期限及 reader。
    /// 参数：snapshot_id、请求身份和最外层 deadline；旧 snapshot 包装签名保留。
    /// 返回：实际归属授权成功或拒权/存储失败；不把客户端 scope 当作归属。
    pub fn authorize_snapshot_until(
        &self,
        snapshot_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: std::time::Instant,
    ) -> Result<(), EngineError> {
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let revision = reader
            .revision_for_snapshot(snapshot_id)?
            .ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
        let ownership = reader.revision_ownership(&revision)?;
        self.authorize_revision_owner_until(ownership, None, principal, authorizer, deadline)?;
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(())
    }

    /// 为可信 TUI 完成一次有界导航或画布读取，允许明确标记的截断画布提交。
    /// 参数：revision/principal/authorizer 为真实请求；consumer 准备完整导航页或绘制画布并给出状态。
    /// 返回：可以提交页面或画布的许可或错误，不返回任意晚到查询数据。
    /// 导航必须给出 Complete 并遵守原读取期限，只有已绘制真实提示的画布可以给出 Truncated。
    /// 图 SQL 永远沿用最初 deadline；末段授权另有固定 50ms 控制窗口，不续租图请求。
    /// 控制锁竞争立即拒绝；SQL 和同步授权回调返回后的期限失败均不能变成部分画布。
    /// 此兼容包装以默认原始字节额度准备目标；实际 TUI 使用 bounded 方法传递整份账本。
    pub fn with_authorized_revision_display_reader(
        &self,
        revision: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline_ms: u64,
        consumer: impl FnOnce(
            &SqliteSnapshotStore,
            &str,
            std::time::Instant,
        ) -> Result<crate::RevisionDisplayCompletion, EngineError>,
    ) -> Result<(), EngineError> {
        self.with_authorized_revision_display_reader_bounded(
            revision,
            principal,
            authorizer,
            QueryBudget {
                deadline_ms,
                ..QueryBudget::default()
            },
            |reader, snapshot, reads| consumer(reader, snapshot, reads.deadline()),
        )
    }

    /// 从真实归属准备到 TUI 节点消费传递同一读取账本；来源：原生 Rust Q-08 / D41。
    /// 参数：revision/身份固定，budget 是整次原始准入额度，consumer 接收已计费账本。
    /// 返回：初始完整准备及末段授权后的提交许可；初始 ByteLimit 不进入画布消费者。
    pub fn with_authorized_revision_display_reader_bounded(
        &self,
        revision: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        budget: QueryBudget,
        consumer: impl FnOnce(
            &SqliteSnapshotStore,
            &str,
            QueryReadBudget,
        ) -> Result<crate::RevisionDisplayCompletion, EngineError>,
    ) -> Result<(), EngineError> {
        let deadline_ms = budget.validated()?.deadline_ms;
        if deadline_ms == 0 || deadline_ms > 1000 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_millis(deadline_ms))
            .ok_or(BusinessError::InvalidArgument)?;
        let expiry = authorizer.expires_at_unix_seconds();
        crate::authority_expiry::check_authority_expiry(expiry)?;
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let mut reads = QueryReadBudget::new(budget, deadline)?;
        let (server, scope) = reader
            .revision_ownership_with_budget(revision, &mut reads)?
            .ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
        let scope = ScopeId::new(scope)
            .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
        let control = self
            .try_control_store()?
            .ok_or(EngineError::Business(BusinessError::BudgetExceeded))?;
        let initial_authorization = control.with_read_deadline(deadline, |control| {
            let withdrawal = crate::request_withdrawal_witness::RequestWithdrawalWitness::capture(
                control, principal, &scope,
            )?;
            if server != control.existing_server_id()?.as_str() {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            if control.scope_revoked(&scope)? {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            withdrawal.check(control)?;
            Ok::<_, EngineError>(withdrawal)
        });
        let withdrawal = match initial_authorization {
            Err(EngineError::Store(error))
                if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                    || error.is_interrupted()
                    || error.is_busy() =>
            {
                return Err(BusinessError::BudgetExceeded.into());
            }
            other => other?,
        };
        crate::authority_expiry::check_authority_expiry(expiry)?;
        drop(control);
        // 初始能力不持控制锁或 SQL handler；两次准入均保持非阻塞语义。
        let decision = authorizer.decide(principal, &Permission::MetadataRead, &scope);
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if matches!(decision, diskgraph_core::Decision::Denied(_)) {
            if let Ok(Some(control)) = self.try_control_store() {
                withdrawal.check(&control)?;
            }
            return Err(BusinessError::PermissionDenied.into());
        }
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        withdrawal.check(&control)?;
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let authorization = control.with_read_deadline(deadline, |control| {
            // 回调可能撤权或改变身份，旧缓存 Allowed 不能替代当前持久授权。
            if server != control.existing_server_id()?.as_str() {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            Self::require_decision_with_control(
                control,
                decision,
                principal,
                &Permission::MetadataRead,
                &scope,
            )?;
            if control.scope_revoked(&scope)? {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            Ok(())
        });
        withdrawal.check(&control)?;
        match authorization {
            Err(EngineError::Store(error))
                if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                    || error.is_interrupted()
                    || error.is_busy() =>
            {
                return Err(BusinessError::BudgetExceeded.into());
            }
            other => other?,
        }
        crate::authority_expiry::check_authority_expiry(expiry)?;
        drop(control);
        // 初次真实授权须在原读取期内完成；未完成的 initial phase 不能借 partial 通路复活。
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let completion = (|| {
            let snapshot_id = reader.revision_snapshot_with_budget(revision, &mut reads)?;
            if std::time::Instant::now() >= deadline {
                return Err(BusinessError::BudgetExceeded.into());
            }
            crate::authority_expiry::check_authority_expiry(expiry)?;
            consumer(&reader, &snapshot_id, reads)
        })();
        let control = self
            .try_control_store()?
            .ok_or(EngineError::Business(BusinessError::BudgetExceeded))?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        let authorization = (|| {
            // Engine 自有 SQL 与能力回调分离；两段 SQL 均保留执行期限和 busy 上界。
            let before_callback = std::time::Instant::now()
                .checked_add(Duration::from_millis(50))
                .ok_or(BusinessError::InvalidArgument)?;
            control.with_read_deadline(before_callback, |control| {
                withdrawal.check(control)?;
                if control.scope_revoked(&scope)? {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
                Ok(())
            })?;
            drop(control);
            // 宿主能力回调不持共享控制锁，也不继承 SQLite progress guard。
            let capability_deadline = std::time::Instant::now()
                .checked_add(Duration::from_millis(50))
                .ok_or(BusinessError::InvalidArgument)?;
            let decision = authorizer.decide(principal, &Permission::MetadataRead, &scope);
            crate::authority_expiry::check_authority_expiry(expiry)?;
            if matches!(decision, diskgraph_core::Decision::Denied(_)) {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            // 回调可改变实时授权；非阻塞重新取得控制锁后复核原撤权见证。
            let control = self
                .try_control_store()?
                .ok_or(EngineError::Business(BusinessError::BudgetExceeded))?;
            withdrawal.check(&control)?;
            // SQL 自身的执行窗口不消耗宿主回调时间；允许结果仍受原能力终检期限约束。
            let after_callback = std::time::Instant::now()
                .checked_add(Duration::from_millis(50))
                .ok_or(BusinessError::InvalidArgument)?;
            control.with_read_deadline(after_callback, |control| {
                let authorization = Self::require_decision_with_control(
                    control,
                    decision,
                    principal,
                    &Permission::MetadataRead,
                    &scope,
                );
                withdrawal.check(control)?;
                authorization?;
                if control.scope_revoked(&scope)?
                    || control.live_permission(principal, &Permission::MetadataRead, &scope)?
                        == Some(false)
                {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
                Ok(())
            })?;
            // 退出原 control SQL guard 后沿同一绝对期限查归属，禁止嵌套 progress guard。
            self.require_terminal_revision_ownership(revision, &scope, &control, after_callback)?;
            // 先保留实时拒权/归属错误；慢回调即使最终允许，也不能提交已准备的 partial。
            if std::time::Instant::now() >= capability_deadline {
                return Err(EngineError::Business(BusinessError::BudgetExceeded));
            }
            Ok(())
        })();
        match authorization {
            Err(EngineError::Store(error))
                if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                    || error.is_interrupted()
                    || error.is_busy() =>
            {
                return Err(BusinessError::BudgetExceeded.into());
            }
            other => other?,
        }
        // 已授权准备和消费的错误也先通过终检；撤权不能被预算错误遮盖。
        let completion = completion?;
        if completion == crate::RevisionDisplayCompletion::Complete
            && std::time::Instant::now() >= deadline
        {
            return Err(BusinessError::BudgetExceeded.into());
        }
        crate::authority_expiry::check_authority_expiry(expiry)?;
        Ok(())
    }
}

impl Engine {
    /// 在控制库锁外取得请求能力，再在锁内读取实时策略并求交集。
    /// 参数：authorizer/principal/permission/scope 指定授权上下文。
    /// 返回：允许或权限/控制库失败。
    pub(super) fn require(
        &self,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        // 固定到期信息在锁外取得；回调、锁等待和授权SQL均不能让迟到允许越过到期。
        let expiry = authorizer.expires_at_unix_seconds();
        let decision = authorizer.decide(principal, permission, scope);
        crate::authority_expiry::check_authority_expiry(expiry)?;
        let control = self.control()?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        Self::require_decision_with_control(&control, decision, principal, permission, scope)?;
        crate::authority_expiry::check_authority_expiry(expiry)
    }
}

impl Engine {
    /// 复用已持有的控制 guard 检查授权，避免重入。
    /// 参数：control 为已有 guard，其他参数为请求能力/主体/权限/范围。
    /// 返回：授权成功或拒绝/存储失败；无持久策略保留可信兼容语义。
    pub(super) fn require_with_control(
        control: &diskgraph_store::ControlStore,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        Self::require_decision_with_control(
            control,
            authorizer.decide(principal, permission, scope),
            principal,
            permission,
            scope,
        )
    }

    /// 验证已取得的请求能力与持久策略交集，不再次调用宿主授权器。
    /// 参数：decision 为同次能力决定，control/主体/权限/范围保持原身份。
    /// 返回：允许、拒权或真实数据库错误；执行期限由调用者的 SQL guard 管理。
    pub(super) fn require_decision_with_control(
        control: &diskgraph_store::ControlStore,
        decision: diskgraph_core::Decision,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        match decision {
            diskgraph_core::Decision::Allowed => {
                // 请求能力只是上限；持久策略存在时，始终与当前数据库授权取交集。
                let denied = if scope == &admin_scope() {
                    control.policy_permission(principal, permission, scope)? == Some(false)
                } else if control.policy_state()?.is_some() {
                    control.live_permission(principal, permission, scope)? == Some(false)
                } else {
                    // 没有持久策略的可信内部兼容入口仍由传入 authorizer 决定。
                    false
                };
                if denied {
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                } else {
                    Ok(())
                }
            }
            diskgraph_core::Decision::Denied(_) => {
                Err(EngineError::Business(BusinessError::PermissionDenied))
            }
        }
    }
}
