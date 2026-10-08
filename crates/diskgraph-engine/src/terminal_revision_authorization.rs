//! revision 读取后终检；来源：OpenSpec SC-04，授权观察不续期数据读取。
use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, ScopeId};
use diskgraph_store::{ControlStore, RevisionOwnershipReader, StoreError};
use std::time::Instant;

impl Engine {
    /// 让已确认的图库隔离拒权优先于未知授权代次冲突。
    /// 参数：result 为已退出 SQL guard 的授权结果，revisions/control/deadline 沿用原观察窗口。
    /// 返回：归属失效拒权；归属有效保留原 Conflict，不刷新期限或复用消费者事务。
    pub(super) fn prioritize_revision_quarantine<T>(
        &self,
        result: Result<T, EngineError>,
        revisions: &[(&str, &ScopeId)],
        control: &ControlStore,
        deadline: Instant,
    ) -> Result<T, EngineError> {
        if matches!(&result, Err(EngineError::Business(BusinessError::Conflict))) {
            self.require_terminal_revision_ownerships(revisions, control, deadline)?;
        }
        result
    }

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
        self.require_terminal_revision_ownerships(&[(revision, scope)], control, deadline)
    }

    /// 在同轮原授权期限内用一个新鲜连接逐侧检查，编码后须重新调用。
    /// 参数：revisions 为首次授权的实际归属，control 为原 guard，deadline 不刷新。
    /// 返回：所有侧均仍匹配；无读事务、不缓存归属，不复用消费者连接。
    pub(super) fn require_terminal_revision_ownerships(
        &self,
        revisions: &[(&str, &ScopeId)],
        control: &ControlStore,
        deadline: Instant,
    ) -> Result<(), EngineError> {
        let result = (|| {
            let server =
                crate::authorization_phase_diagnostic::observe("terminal_server_sql", || {
                    Ok(control
                        .with_read_deadline(deadline, |control| control.existing_server_id())?)
                })?;
            #[cfg(test)]
            let reader_started = Instant::now();
            let reader =
                crate::authorization_phase_diagnostic::observe("terminal_reader_open", || {
                    Ok(RevisionOwnershipReader::open_until(
                        &self.graph_path,
                        deadline,
                    )?)
                })?;
            #[cfg(test)]
            crate::relation_query_diagnostics_tests::detail("terminal_reader_open", reader_started);
            #[cfg(test)]
            crate::relation_request_tests::terminal_reader_opened();
            #[cfg(test)]
            let ownership_started = Instant::now();
            for (revision, scope) in revisions {
                // 每次 SELECT 独立观察当前 WAL；不能开启冻结两侧归属的事务。
                crate::authorization_phase_diagnostic::observe("terminal_ownership_sql", || {
                    if !reader.matches(revision, server.as_str(), scope.as_str())? {
                        return Err(BusinessError::PermissionDenied.into());
                    }
                    if Instant::now() >= deadline {
                        return Err(BusinessError::BudgetExceeded.into());
                    }
                    Ok(())
                })?;
            }
            #[cfg(test)]
            crate::relation_query_diagnostics_tests::detail(
                "terminal_ownership_sql",
                ownership_started,
            );
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
