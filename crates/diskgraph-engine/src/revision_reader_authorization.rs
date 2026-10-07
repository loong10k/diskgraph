//! 通用可信 revision reader 的初始与终态授权边界。
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, Permission, PrincipalId, ScopeId};
use diskgraph_store::SqliteSnapshotStore;
use std::time::Duration;

impl Engine {
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
        let expiry = authorizer.expires_at_unix_seconds();
        crate::authority_expiry::check_authority_expiry(expiry)?;
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let (server, scope) = reader
            .revision_ownership(revision_id)?
            .ok_or(BusinessError::PermissionDenied)?;
        let scope = ScopeId::new(scope).map_err(|_| BusinessError::PermissionDenied)?;
        let control = self.control_until(deadline)?;
        // 在首次授权前绑定实际依赖；所有 SQL 仍使用请求最初的截止时间。
        let withdrawal = control
            .with_read_deadline(deadline, |control| {
                let withdrawal =
                    crate::request_withdrawal_witness::RequestWithdrawalWitness::capture(
                        control, principal, &scope,
                    )?;
                if server != control.existing_server_id()?.as_str() {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
                withdrawal.check(control)?;
                Ok::<_, EngineError>(withdrawal)
            })
            .map_err(|error| match error {
                EngineError::Store(error)
                    if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                        || error.is_interrupted()
                        || error.is_busy() =>
                {
                    EngineError::Business(BusinessError::BudgetExceeded)
                }
                other => other,
            })?;
        drop(control);
        let authorization =
            self.require_reader_capability_until(authorizer, principal, &scope, expiry, deadline);
        if let Err(error) = authorization {
            // 失败不再等待原期限锁；可立即取得的见证仍检查，但不以锁预算覆盖已观察拒权。
            if let Ok(Some(control)) = self.try_control_store() {
                withdrawal.check(&control)?;
            }
            return Err(error);
        }
        let control = self.control_until(deadline)?;
        withdrawal.check(&control)?;
        let current_server = control
            .with_read_deadline(deadline, |control| control.existing_server_id())
            .map_err(|error| {
                if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                    || error.is_interrupted()
                    || error.is_busy()
                {
                    EngineError::Business(BusinessError::BudgetExceeded)
                } else {
                    EngineError::Store(error)
                }
            })?;
        if server != current_server.as_str() {
            return Err(BusinessError::PermissionDenied.into());
        }
        withdrawal.check(&control)?;
        // 即使短 SQL 未触发 VM handler，初始授权过期也不能进入消费者。
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        drop(control);
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        let result = consumer(&reader, &snapshot_id, deadline)?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        // 撤销与单项权限在同一控制库锁下复核，可信兼容模式同样不能越过撤销。
        let control = self.control_until(deadline)?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        withdrawal.check(&control)?;
        let before_callback = std::time::Instant::now()
            .checked_add(Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        control
            .with_read_deadline(before_callback, |control| {
                if control.scope_revoked(&scope)? {
                    return Err(BusinessError::PermissionDenied.into());
                }
                Ok::<(), EngineError>(())
            })
            .map_err(reader_terminal_control_error)?;
        drop(control);
        // 宿主终检能力不持共享控制锁；固定能力期限与SQL观察窗口分别沿既有50ms规则计量。
        let capability_deadline = std::time::Instant::now()
            .checked_add(Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        let decision = authorizer.decide(principal, &Permission::MetadataRead, &scope);
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if matches!(decision, diskgraph_core::Decision::Denied(_)) {
            return Err(BusinessError::PermissionDenied.into());
        }
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        withdrawal.check(&control)?;
        let after_callback = std::time::Instant::now()
            .checked_add(Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        let authorization = control
            .with_read_deadline(after_callback, |control| {
                Self::require_decision_with_control(
                    control,
                    decision,
                    principal,
                    &Permission::MetadataRead,
                    &scope,
                )?;
                if control.scope_revoked(&scope)? {
                    return Err(BusinessError::PermissionDenied.into());
                }
                Ok::<(), EngineError>(())
            })
            .map_err(reader_terminal_control_error);
        withdrawal.check(&control)?;
        authorization?;
        let timely = std::time::Instant::now() < capability_deadline;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        self.require_terminal_revision_ownership(revision_id, &scope, &control, deadline)?;
        if !timely || std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        crate::authority_expiry::check_authority_expiry(expiry)?;
        Ok(result)
    }
}

// 终检控制SQL错误仅映射执行预算、busy及中断，损坏和拒权保持原语义。
fn reader_terminal_control_error(error: EngineError) -> EngineError {
    match error {
        EngineError::Store(error)
            if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                || error.is_interrupted()
                || error.is_busy() =>
        {
            EngineError::Business(BusinessError::BudgetExceeded)
        }
        other => other,
    }
}
