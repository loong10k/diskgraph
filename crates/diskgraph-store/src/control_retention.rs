//! 控制库历史引用保护事务；与图数据回收保持独立持久性边界。
use crate::{ControlStore, Result};
use diskgraph_core::ScopeId;

impl ControlStore {
    /// 操作记录未绑定 revision 的旧模型保守保留整个 scope；事务阻止并发新引用。
    /// 固定计划、操作和恢复引用执行历史治理，不承诺跨库原子事务。
    /// 参数：scope_id：实际所属范围 ID；work：有效事务期间的内部回调。
    /// 返回：回调结果；回调收到的 bool 表示没有保留引用；错误不提交控制事务。
    pub fn with_retention_guard<T>(
        &mut self,
        scope_id: &ScopeId,
        work: impl FnOnce(bool) -> Result<T>,
    ) -> Result<T> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let referenced: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM plans WHERE scope_id = ?1 UNION ALL SELECT 1 FROM operations WHERE scope_id = ?1 UNION ALL SELECT 1 FROM recovery_entries WHERE scope_id = ?1)", [scope_id.as_str()], |row| row.get(0))?;
        let result = work(!referenced)?;
        tx.commit()?;
        Ok(result)
    }
}
