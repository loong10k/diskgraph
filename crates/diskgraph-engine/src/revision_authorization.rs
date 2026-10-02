//! 共享 Engine 的 revision_authorization 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError, admin_scope};
use diskgraph_core::{Authorizer, BusinessError, Permission, PrincipalId, ScopeId};
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
    /// 共用 reader/期限执行授权读取并在返回前复检。
    /// 参数：revision、请求身份、deadline_ms 与 consumer 指定读取范围。
    /// 返回：consumer 结果或授权/存储失败；consumer 仅可读取获准快照。
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
        let deadline = std::time::Instant::now() + Duration::from_millis(deadline_ms);
        let reader = SqliteSnapshotStore::open_reader(&self.graph_path, deadline_ms, None)?;
        let scope =
            self.authorize_revision_with_reader(&reader, None, revision_id, principal, authorizer)?;
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        let result = consumer(&reader, &snapshot_id, deadline)?;
        // 撤销与单项权限在同一控制库锁下复核，可信兼容模式同样不能越过撤销。
        let control = self.control_store()?;
        if control.scope(&scope)?.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        Self::require_with_control(
            &control,
            authorizer,
            principal,
            &Permission::MetadataRead,
            &scope,
        )?;
        Ok(result)
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
