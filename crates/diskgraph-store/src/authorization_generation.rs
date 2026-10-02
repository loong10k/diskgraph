//! 授权数据变更的持久计数，独立于不会随单条 grant 改变的策略 epoch。

use crate::{ControlStore, Result};

impl ControlStore {
    /// 读取授权变更计数，覆盖策略、grant、scope 的跨连接提交。
    /// 参数：无额外输入，使用当前控制库连接。
    /// 返回：单调计数，缺失或损坏时失败关闭。
    pub fn authorization_generation(&self) -> Result<u64> {
        let generation: i64 = self.connection.query_row(
            "SELECT generation FROM authorization_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        u64::try_from(generation)
            .map_err(|_| crate::StoreError::InvalidGraph("invalid authorization generation".into()))
    }

    /// 带绝对期限读取授权计数，同时限制 SQLite 锁等待与执行进度。
    /// 参数：deadline 为单调时钟截止点；调用期间独占本连接的执行回调。
    /// 返回：计数或预算/数据库错误，还原实际 busy_timeout 并清除本函数回调。
    /// 不得与 crate 内其他自定义 progress_handler 混用；原始连接不对外公开。
    pub fn authorization_generation_until(&self, deadline: std::time::Instant) -> Result<u64> {
        if std::time::Instant::now() >= deadline {
            return Err(crate::StoreError::BudgetExceeded);
        }
        self.connection
            .progress_handler(1, Some(move || std::time::Instant::now() >= deadline))?;
        // busy_timeout 是连接本地 PRAGMA，无业务表读锁；读取真实值而非假设默认值。
        let previous: std::result::Result<i64, rusqlite::Error> =
            self.connection
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0));
        let previous = match previous {
            Ok(timeout) => std::time::Duration::from_millis(timeout.max(0) as u64),
            Err(error) => {
                self.connection.progress_handler(0, None::<fn() -> bool>)?;
                return Err(error.into());
            }
        };
        if let Err(error) = self
            .connection
            .busy_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        {
            self.connection.progress_handler(0, None::<fn() -> bool>)?;
            return Err(error.into());
        }
        let result = self.authorization_generation();
        // 此连接在 guard 下独占；清理不能把下一次正常写入留在已过期 handler 上。
        let cleared = self.connection.progress_handler(0, None::<fn() -> bool>);
        let restored = self.connection.busy_timeout(previous);
        cleared?;
        restored?;
        let lock_budget_exhausted = matches!(&result, Err(crate::StoreError::Sqlite(rusqlite::Error::SqliteFailure(error, _))) if error.code == rusqlite::ErrorCode::DatabaseBusy);
        if std::time::Instant::now() >= deadline || lock_budget_exhausted {
            return Err(crate::StoreError::BudgetExceeded);
        }
        result
    }

    /// 安装控制库 v6 的原子计数及触发器。
    /// 参数：connection 为待升级连接；返回：提交结果，失败时事务回滚。
    pub(crate) fn migrate_authorization_generation(
        connection: &rusqlite::Connection,
    ) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        transaction.execute_batch(
            "CREATE TABLE authorization_state (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 generation INTEGER NOT NULL CHECK (typeof(generation) = 'integer' AND generation >= 0)
             );
             INSERT INTO authorization_state VALUES (1, 0);",
        )?;
        // 与权限变更处于同一事务；回滚不改变计数，任务/操作表更新不导致误撤权。
        // 全部名称是编译期固定值，没有来自请求的 SQL 标识符。
        for table in ["policy", "grants", "scopes"] {
            for event in ["insert", "update", "delete"] {
                transaction.execute_batch(&format!(
                    "CREATE TRIGGER auth_generation_{table}_{event} AFTER {event} ON {table}
                     BEGIN UPDATE authorization_state SET generation = generation + 1 WHERE id = 1; END;"
                ))?;
            }
        }
        transaction.pragma_update(None, "user_version", 6)?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "authorization_generation_tests.rs"]
mod tests;
