//! 根目录最新快照的授权窄读，空结果同样绑定真实范围撤权见证。
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, Locator, PrincipalId, QueryBudget, QueryReadBudget,
};
use diskgraph_store::{ControlStore, RevisionOwnershipReader, SqliteSnapshotStore, StoreError};
use std::time::Instant;

impl Engine {
    /// 编码指定已注册根的最新快照标识，并在释放结果前复验实际归属。
    /// 参数：root/主体/能力固定请求身份，budget/deadline 为原预算，consume 仅收到有限标识。
    /// 返回：编码结果或权限、预算、存储错误；空结果不得绕过撤权或期限。
    pub fn with_latest_snapshot_id_until<T>(
        &self,
        root: &Locator,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        budget: QueryBudget,
        deadline: Instant,
        consume: impl FnOnce(Option<&str>, &mut QueryReadBudget) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let expiry = authorizer.expires_at_unix_seconds();
        crate::authority_expiry::check_authority_expiry(expiry)?;
        let mut reads = QueryReadBudget::new(budget, deadline)?;
        if !reads.admit(0, 0, root.raw_b64.len()) {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let control = self.control_until(deadline)?;
        let (scope, server, withdrawal, generation) = control
            .with_read_deadline(deadline, |control| {
                let scope = control
                    .scope_id_for_root_with_budget(root, &mut reads)?
                    .ok_or(BusinessError::PermissionDenied)?;
                let server = bounded_server(control, &mut reads)?;
                let withdrawal =
                    crate::request_withdrawal_witness::RequestWithdrawalWitness::capture(
                        control, principal, &scope,
                    )?;
                Ok::<_, EngineError>((
                    scope,
                    server,
                    withdrawal,
                    control.authorization_generation()?,
                ))
            })
            .map_err(budget_error)?;
        drop(control);
        self.require_reader_capability_until(authorizer, principal, &scope, expiry, deadline)?;
        let mut selected = None;
        let result = (|| {
            let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
            selected = reader.latest_snapshot_ids_with_budget(
                server.as_str(),
                scope.as_str(),
                &mut reads,
            )?;
            consume(
                selected.as_ref().map(|(_, snapshot)| snapshot.as_str()),
                &mut reads,
            )
        })();
        // 编码后先读取负向见证；已知撤权不能被原请求到期或 SQL 预算覆盖。
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        withdrawal.check(&control)?;
        drop(control);
        self.require_reader_capability_until(authorizer, principal, &scope, expiry, deadline)?;
        let terminal =
            (Instant::now() + crate::terminal_authorization_windows::DATABASE_WINDOW).min(deadline);
        let control = self.control_until(terminal)?;
        withdrawal.check(&control)?;
        let terminal_control = control.with_read_deadline(terminal, |control| {
            withdrawal.check(control)?;
            if !withdrawal.has_native_watch() && control.authorization_generation()? != generation {
                return Err(BusinessError::Conflict.into());
            }
            if bounded_server(control, &mut reads)? != server
                || control
                    .scope_id_for_root_with_budget(root, &mut reads)?
                    .as_ref()
                    != Some(&scope)
            {
                return Err(BusinessError::PermissionDenied.into());
            }
            Ok::<_, EngineError>(())
        });
        withdrawal.check(&control)?;
        terminal_control.map_err(budget_error)?;
        // 新鲜连接不复用消费者 WAL 快照，也不缓存已允许归属。
        let terminal_ownership = (|| {
            if let Some((revision, _)) = selected.as_ref() {
                let ownership = RevisionOwnershipReader::open_until(&self.graph_path, terminal)?;
                if !ownership.matches(revision, server.as_str(), scope.as_str())? {
                    return Err(BusinessError::PermissionDenied.into());
                }
            }
            Ok::<_, EngineError>(())
        })();
        withdrawal.check(&control)?;
        terminal_ownership.map_err(budget_error)?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if Instant::now() >= terminal {
            return Err(BusinessError::BudgetExceeded.into());
        }
        result.map_err(budget_error)
    }
}

// 持久服务器身份同样先准入再拥有，不能由损坏的大字段绕过本次窄读账本。
fn bounded_server(
    control: &ControlStore,
    reads: &mut QueryReadBudget,
) -> Result<diskgraph_core::ServerId, EngineError> {
    Ok(control.existing_server_id_with_admission(&mut |raw, _, _| {
        let bytes = usize::try_from(raw).map_err(|_| StoreError::BudgetExceeded)?;
        if reads.admit(0, 0, bytes) {
            Ok(())
        } else {
            Err(StoreError::BudgetExceeded)
        }
    })?)
}

fn budget_error(error: EngineError) -> EngineError {
    match error {
        EngineError::Store(e)
            if matches!(e, StoreError::BudgetExceeded) || e.is_busy() || e.is_interrupted() =>
        {
            BusinessError::BudgetExceeded.into()
        }
        other => other,
    }
}
