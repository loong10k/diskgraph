use crate::{Result, StoreError};
use rusqlite::{Connection, OpenFlags, params};
use std::path::Path;
use std::time::{Duration, Instant};

/// 终检专用独立归属连接；来源：原生Rust SC-04/Q-08，无Java对等对象。
/// 不交给消费者、不提供事务或广域查询，只按实际过滤视图读取布尔结果。
/// 可信内部接口，不代替请求主体、scope授权或数据库启动时的schema校验。
pub struct RevisionOwnershipReader {
    connection: Connection,
    deadline: Instant,
}

impl RevisionOwnershipReader {
    /// 参数：path为引擎固定图库路径，deadline为原终检期限；返回：独立只读连接或原错误。
    /// 不迁移、不创建数据库，不配置通用节点缓存或临时存储；同步打开仅协作受限。
    pub fn open_until(path: &Path, deadline: Instant) -> Result<Self> {
        check(deadline)?;
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        check(deadline)?;
        // SQLite 默认 busy 回调累计的是请求 sleep 毫秒数，不是单调时钟实耗；
        // Windows 的睡眠粒度/调度可令短窗口显著超时。终检不重试锁竞争，直接失败关闭。
        connection.busy_timeout(Duration::ZERO)?;
        connection.progress_handler(1000, Some(move || Instant::now() >= deadline))?;
        check(deadline)?;
        Ok(Self {
            connection,
            deadline,
        })
    }

    /// 参数：revision/server/scope为首次真实授权身份；返回：当前匹配布尔值或原SQL/期限失败。
    /// 未绑定、隔离和不匹配均为false；真实拒绝先于查询结束后的期限，允许结果须仍在原期限内。
    pub fn matches(&self, revision: &str, server: &str, scope: &str) -> Result<bool> {
        check(self.deadline)?;
        let matches: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM revision_authorized_ownership WHERE revision_id=?1 AND server_id=?2 AND scope_id=?3)",
            params![revision, server, scope], |row| row.get(0),
        )?;
        if matches {
            check(self.deadline)?;
        }
        Ok(matches)
    }
}

fn check(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(StoreError::BudgetExceeded)
    } else {
        Ok(())
    }
}
