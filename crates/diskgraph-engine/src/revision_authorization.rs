//! 共享 Engine 的 revision_authorization 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError, admin_scope};
use diskgraph_core::{
    Authorizer, BusinessError, Permission, PrincipalId, QueryBudget, QueryReadBudget, ScopeId,
};
use diskgraph_store::SqliteSnapshotStore;
use std::time::Duration;

impl Engine {
    /// 将 revision 的实际归属检查用于元数据读取。
    /// 参数：revision_id 与 principal/authorizer 为请求身份。
    /// 返回：授权成功或拒绝/归属读取失败。
    pub(super) fn require_read_for_revision(
        &self,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        self.authorize_revision(None, revision_id, principal, authorizer)
            .map(|_| ())
    }
}

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
        self.authorize_revision_with_reader(
            &reader,
            expected_scope,
            revision_id,
            principal,
            authorizer,
        )
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
        self.authorize_revision_owner(ownership, expected_scope, principal, authorizer)
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
        self.authorize_revision_with_reader(&reader, None, &revision, principal, authorizer)
            .map(|_| ())
    }

    /// 共用 reader/期限执行授权读取并在返回前复检。
    /// 参数：revision、请求身份、deadline_ms 与 consumer 指定读取范围。
    /// 返回：consumer 结果或授权/存储失败；末段期限耗尽返回 BudgetExceeded，consumer 仅可读取获准快照。
    /// 在一次授权读取中复用独立 SQLite 连接和共同截止时间。
    /// 参数为实际 revision、请求主体、能力授权器和 1–1000 毫秒预算；
    /// 消费者仅接收该 revision 的快照 ID，返回前再次检查实时元数据权限。
    /// 供可信本机展示适配器使用，消费者不得查询其他快照。
    pub fn with_authorized_revision_reader<T>(
        &self,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline_ms: u64,
        consumer: impl FnOnce(&SqliteSnapshotStore, &str, std::time::Instant) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        if !(1..=1000).contains(&deadline_ms) {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_millis(deadline_ms))
            .ok_or(BusinessError::InvalidArgument)?;
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let (server, scope) = reader
            .revision_ownership(revision_id)?
            .ok_or(BusinessError::PermissionDenied)?;
        let scope = ScopeId::new(scope).map_err(|_| BusinessError::PermissionDenied)?;
        let control = self.control_store()?;
        // 在首次授权前绑定实际依赖；所有 SQL 仍使用请求最初的截止时间。
        let withdrawal = control.with_read_deadline(deadline, |control| {
            let withdrawal = crate::request_withdrawal_witness::RequestWithdrawalWitness::capture(
                control, principal, &scope,
            )?;
            if server != control.existing_server_id()?.as_str() {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            let authorization =
                Self::require_terminal_relation(control, authorizer, principal, &scope);
            withdrawal.check(control)?;
            authorization?;
            Ok::<_, EngineError>(withdrawal)
        })?;
        drop(control);
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        let result = consumer(&reader, &snapshot_id, deadline)?;
        // 撤销与单项权限在同一控制库锁下复核，可信兼容模式同样不能越过撤销。
        let control = self.control_store()?;
        withdrawal.check(&control)?;
        let authorization =
            Self::require_terminal_relation(&control, authorizer, principal, &scope);
        withdrawal.check(&control)?;
        authorization?;
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(result)
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
            let authorization =
                Self::require_terminal_relation(control, authorizer, principal, &scope);
            withdrawal.check(control)?;
            authorization?;
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
            consumer(&reader, &snapshot_id, reads)
        })();
        // 归属来自读取前解析的不可变已发布 revision，不在过期 graph reader 上追加查询。
        let authorization_deadline = std::time::Instant::now()
            .checked_add(Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        let control = self
            .try_control_store()?
            .ok_or(EngineError::Business(BusinessError::BudgetExceeded))?;
        withdrawal.check(&control)?;
        let authorization = control.with_read_deadline(authorization_deadline, |control| {
            let authorization =
                Self::require_terminal_relation(control, authorizer, principal, &scope);
            withdrawal.check(control)?;
            authorization?;
            // 能力回调之后纯读实际持久 grant；guard 不冻结独立数据库连接的撤权。
            if control.scope_revoked(&scope)?
                || control.live_permission(principal, &Permission::MetadataRead, &scope)?
                    == Some(false)
            {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            Ok(())
        });
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
        withdrawal.check(&control)?;
        // 已授权准备和消费的错误也先通过终检；撤权不能被预算错误遮盖。
        let completion = completion?;
        if completion == crate::RevisionDisplayCompletion::Complete
            && std::time::Instant::now() >= deadline
        {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(())
    }
}

impl Engine {
    /// 在控制库锁下求请求能力与实时策略交集。
    /// 参数：authorizer/principal/permission/scope 指定授权上下文。
    /// 返回：允许或权限/控制库失败。
    pub(super) fn require(
        &self,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        let control = self.control()?;
        Self::require_with_control(&control, authorizer, principal, permission, scope)
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
        match authorizer.decide(principal, permission, scope) {
            diskgraph_core::Decision::Allowed => {
                // 请求能力只是上限；持久策略存在时，始终与当前数据库授权取交集。
                let denied = if scope == &admin_scope() {
                    control.policy_state()?.is_some()
                        && !matches!(
                            control.authorizer()?.decide(principal, permission, scope),
                            diskgraph_core::Decision::Allowed
                        )
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
