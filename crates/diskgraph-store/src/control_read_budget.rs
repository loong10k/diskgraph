//! 既有控制连接的有限只读阶段，不创建第二个控制库 owner。

use crate::{ControlStore, StoreError};
use diskgraph_core::ServerId;
use rusqlite::OptionalExtension;
use std::time::{Duration, Instant};

impl ControlStore {
    /// 只读已初始化的持久 server 身份，不在有期限授权阶段创建新身份。
    /// 参数：无。返回：按原 ServerId 规则校验的身份；缺失或损坏时失败关闭。
    pub fn existing_server_id(&self) -> crate::Result<ServerId> {
        let id: String = self
            .connection
            .prepare_cached("SELECT server_id FROM server WHERE id=1")?
            .query_row([], |row| row.get(0))
            .optional()?
            .ok_or_else(|| StoreError::InvalidGraph("stored server id is missing".into()))?;
        ServerId::new(id)
            .map_err(|error| StoreError::InvalidGraph(format!("stored server id invalid: {error}")))
    }

    /// 在独占既有 guard 下临时限制控制库 SQL，并完整还原本连接配置。
    /// 参数：deadline 为该控制阶段的固定绝对期限，consumer 只进行读取和授权决定。
    /// 返回：读取结果或错误；同步 consumer 仅可在返回后检查时间，不保证硬实时。
    /// 控制连接不公开自定义回调入口，本函数不保存未知 progress_handler；必须没有其他活跃回调。
    /// 不得嵌套其他期限函数；调用方负责非阻塞获取同一控制 guard，展开 panic 前也还原配置。
    pub fn with_read_deadline<T, E: From<StoreError>>(
        &self,
        deadline: Instant,
        consumer: impl FnOnce(&Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        if Instant::now() >= deadline {
            return Err(StoreError::BudgetExceeded.into());
        }
        // busy_timeout 是连接本地值；记录实际配置，不把默认值写回覆盖宿主设置。
        let previous: i64 = self
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .map_err(StoreError::from)?;
        let previous = Duration::from_millis(previous.max(0) as u64);
        self.connection
            .busy_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(StoreError::from)?;
        if let Err(error) = self
            .connection
            .progress_handler(1, Some(move || Instant::now() >= deadline))
        {
            self.connection
                .busy_timeout(previous)
                .map_err(StoreError::from)?;
            return Err(StoreError::from(error).into());
        }
        // 连接和 closure 可能不实现 UnwindSafe；仅捕获以还原配置，然后继续原始 panic。
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // 配置语句准备与回调安装也消耗原窗口；不能先执行过期消费者再拒绝其结果。
            // 仍经下方统一还原连接配置，不在过期分支提前返回或刷新期限。
            if Instant::now() >= deadline {
                Err(StoreError::BudgetExceeded.into())
            } else {
                consumer(self)
            }
        }));
        let expired = Instant::now() >= deadline;
        // 错误路径同样清除 handler，防止下一次正常控制读写继承过期期限。
        let cleared = self.connection.progress_handler(0, None::<fn() -> bool>);
        let restored = self.connection.busy_timeout(previous);
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                // 两项还原已经实际执行；保留原 panic，不把它伪装成预算或成功。
                std::panic::resume_unwind(payload);
            }
        };
        cleared.map_err(StoreError::from)?;
        restored.map_err(StoreError::from)?;
        match result {
            Ok(_) if expired => Err(StoreError::BudgetExceeded.into()),
            other => other,
        }
    }
}
