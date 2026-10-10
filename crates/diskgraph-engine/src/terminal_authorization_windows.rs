//! 请求终检的独立授权窗口；不续期原数据查询账本。
use std::time::Duration;

/// 数据库终检窗口：控制 SQL 与新鲜归属观察共用原阶段的绝对期限。
pub(super) const DATABASE_WINDOW: Duration = Duration::from_millis(250);

/// 同步能力回调的合作接受窗口；迟到允许仍须拒绝，不承诺硬抢占。
pub(super) const CAPABILITY_WINDOW: Duration = Duration::from_millis(50);
