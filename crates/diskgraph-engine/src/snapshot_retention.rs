//! 共享 Engine 的 snapshot_retention 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError, admin_scope};
use diskgraph_core::{Authorizer, BusinessError, Permission, PrincipalId, ScopeId};

impl Engine {
    /// 按范围元数据授权分页列出已发布快照。
    /// 参数：scope_id/principal/authorizer 绑定范围身份，limit/offset 为分页。
    /// 返回：快照列表或授权/查询失败。
    /// Lists snapshots for a scope's root (C05 list/show), newest first.
    pub fn list_snapshots(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<diskgraph_core::DiskSnapshot>, EngineError> {
        self.require(authorizer, principal, &Permission::MetadataRead, scope_id)?;
        if self.scope(scope_id)?.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        let server = self.server_id()?;
        Ok(self.revision_reader()?.scope_snapshots(
            server.as_str(),
            scope_id.as_str(),
            limit,
            offset,
        )?)
    }
}

impl Engine {
    /// 在原请求期限内分页读取实际 scope 快照，并复验实时授权与负向撤权见证。
    /// 参数：scope_id/principal/authorizer 为请求归属与能力，limit/offset 为分页，deadline 为原期限。
    /// 返回：快照页或授权/预算/数据库错误；不为读取重建时间预算。
    pub fn list_snapshots_until(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        limit: u64,
        offset: u64,
        deadline: std::time::Instant,
    ) -> Result<Vec<diskgraph_core::DiskSnapshot>, EngineError> {
        let expiry = authorizer.expires_at_unix_seconds();
        crate::authority_expiry::check_authority_expiry(expiry)?;
        let control = self.control_until(deadline)?;
        let (server, withdrawal, generation) = control.with_read_deadline(deadline, |control| {
            control.scope(scope_id)?;
            Ok::<_, EngineError>((
                control.existing_server_id()?,
                crate::request_withdrawal_witness::RequestWithdrawalWitness::capture(
                    control, principal, scope_id,
                )?,
                control.authorization_generation()?,
            ))
        })?;
        drop(control);
        self.require_reader_capability_until(authorizer, principal, scope_id, expiry, deadline)?;
        let reader = diskgraph_store::SqliteSnapshotStore::open_reader_until(
            &self.graph_path,
            deadline,
            None,
        )?;
        let snapshots =
            reader.scope_snapshots(server.as_str(), scope_id.as_str(), limit, offset)?;
        self.require_reader_capability_until(authorizer, principal, scope_id, expiry, deadline)?;
        let control = self.control_until(deadline)?;
        control.with_read_deadline(deadline, |control| {
            withdrawal.check(control)?;
            // 未知原生通知能力不能把撤权再恢复误当持续允许；代次变化只证明冲突。
            if !withdrawal.has_native_watch() && control.authorization_generation()? != generation {
                return Err(BusinessError::Conflict.into());
            }
            if control.existing_server_id()? != server {
                return Err(BusinessError::PermissionDenied.into());
            }
            Ok::<_, EngineError>(())
        })?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        Ok(snapshots)
    }
}

impl Engine {
    /// 预览或应用范围历史回收并保护持久操作引用。
    /// 参数：scope_id、keep_last、apply 指定保留策略；principal/authorizer 为请求身份。
    /// 返回：候选 revision；引用不明确时空列表；失败返回错误。
    /// 回收 scope 旧历史；无 apply 时仅预览，引用关系不明确时保守保留。
    pub fn prune_snapshots(
        &self,
        scope_id: &ScopeId,
        keep_last: u64,
        apply: bool,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<diskgraph_store::RevisionRecord>, EngineError> {
        self.require(authorizer, principal, &Permission::IndexWrite, scope_id)?;
        let server = self.server_id()?;
        let mut graph = self.graph()?;
        let mut control = self.control()?;
        if control.scope(scope_id)?.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        Ok(control.with_retention_guard(scope_id, |safe| {
            if safe {
                graph.prune_revisions(server.as_str(), scope_id.as_str(), keep_last, apply)
            } else {
                Ok(Vec::new())
            }
        })?)
    }
}

impl Engine {
    /// 在既有管理写权限门禁后更新快照 pin。
    /// 参数：snapshot_id/pinned 指定对象与 pin；principal/authorizer 为请求身份。
    /// 返回：更新成功或授权/存储失败。
    /// Pins or unpins one snapshot (C05 pin, retention protection).
    pub fn pin_snapshot(
        &self,
        snapshot_id: &str,
        pinned: bool,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        self.require_write_for_snapshot(snapshot_id, principal, authorizer)?;
        self.graph()?.pin_snapshot(snapshot_id, pinned)?;
        Ok(())
    }
}

impl Engine {
    /// 按既有管理写权限删除可回收快照，不触碰用户文件。
    /// 参数：snapshot_id 为对象；principal/authorizer 为请求身份。
    /// 返回：删除成功或 pin/引用/授权失败。
    /// Removes one snapshot from graph history (C05 remove). Refused when
    /// pinned or referenced by published revisions; never touches user files.
    pub fn remove_snapshot(
        &self,
        snapshot_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        self.require_write_for_snapshot(snapshot_id, principal, authorizer)?;
        self.graph()?.remove_snapshot(snapshot_id)?;
        Ok(())
    }
}

impl Engine {
    /// 保留既有快照管理入口的 admin 范围 index:write 门禁。
    /// 参数：snapshot_id 保留旧签名，principal/authorizer 为管理请求身份。
    /// 返回：管理写权限允许或授权错误；普通范围写权限不足以管理共享索引。
    fn require_write_for_snapshot(
        &self,
        _snapshot_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        // P1: retention edits authorize against the admin scope because the
        // graph index is shared across scopes; scope-scoped retention arrives
        // with per-scope graph namespaces (ST-04 full semantics, P2).
        self.require(
            authorizer,
            principal,
            &Permission::IndexWrite,
            &admin_scope(),
        )
    }
}
