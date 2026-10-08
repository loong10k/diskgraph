//! 同一控制连接的有界写事务；来源：原生 Rust D44 / EC-04，非另一个持久状态 owner。
use crate::{Result, StoreError};
use rusqlite::Connection;
use std::cell::Cell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 独占控制 guard 内的原期限与事务清理；来源：DiskGraph 原生 Rust 入队协议。
/// 进入时无活动事务/自定义 progress 回调，不允许嵌套。释放顺序为清回调、回滚、恢复宿主 busy。
pub(crate) struct ControlWriteDeadline<'a> {
    connection: &'a Connection,
    deadline: Instant,
    expiry: Option<u64>,
    previous_busy: Duration,
    progress_installed: Cell<bool>,
    finished: Cell<bool>,
}
impl<'a> ControlWriteDeadline<'a> {
    /// 参数：唯一现有连接、原请求 Instant 与原绝对认证到期；返回：配置守卫或真实拒绝。
    pub(crate) fn new(
        connection: &'a Connection,
        deadline: Instant,
        expiry: Option<u64>,
    ) -> Result<Self> {
        Self::check_limits(deadline, expiry)?;
        if !connection.is_autocommit() {
            return Err(StoreError::Conflict(
                "nested bounded control transaction".into(),
            ));
        }
        let previous: i64 = connection.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
        let guard = Self {
            connection,
            deadline,
            expiry,
            previous_busy: Duration::from_millis(previous.max(0) as u64),
            progress_installed: Cell::new(false),
            finished: Cell::new(false),
        };
        // VM 仅检查原单调期限；原认证按真实绝对 Unix 时间在每个阶段和提交前复验。
        connection.progress_handler(1, Some(move || Instant::now() >= deadline))?;
        guard.progress_installed.set(true);
        guard.check()?;
        Ok(guard)
    }

    fn check_limits(deadline: Instant, expiry: Option<u64>) -> Result<()> {
        if Instant::now() >= deadline {
            return Err(StoreError::BudgetExceeded);
        }
        if let Some(expiry) = expiry {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| StoreError::Conflict("job authority clock unavailable".into()))?;
            if now.as_secs() >= expiry {
                return Err(StoreError::Conflict("job request authority expired".into()));
            }
        }
        Ok(())
    }

    /// 参数：无；返回：原请求与原认证仍有效，绝不重建期限或续期。
    pub(crate) fn check(&self) -> Result<()> {
        Self::check_limits(self.deadline, self.expiry)
    }

    fn remaining(&self) -> Result<Duration> {
        self.check()?;
        let mut remaining = self.deadline.saturating_duration_since(Instant::now());
        if let Some(expiry) = self.expiry {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| StoreError::Conflict("job authority clock unavailable".into()))?;
            let authentication = Duration::from_secs(expiry).saturating_sub(now);
            remaining = remaining.min(authentication);
        }
        Ok(remaining)
    }

    fn run_sql<T>(&self, mut operation: impl FnMut() -> Result<T>) -> Result<T> {
        loop {
            let remaining = self.remaining()?;
            let limited_by_request = remaining < self.previous_busy;
            self.connection
                .busy_timeout(remaining.min(self.previous_busy))?;
            self.check()?;
            match operation() {
                Ok(value) => return Ok(value),
                Err(error) if error.is_busy() || error.is_interrupted() => {
                    // 只有确切的 BUSY/INTERRUPTED 与真实期限失败对应时才转预算/到期。
                    self.check()?;
                    let remaining = self.remaining()?;
                    if error.is_busy() && limited_by_request && remaining < Duration::from_millis(1)
                    {
                        // SQLite 毫秒 floor 可在真实期限前返回；最多等这不足1ms并复检原期限。
                        // 宿主自身短 busy timeout 与其他立即 BUSY 错误不会进入该分支。
                        std::thread::sleep(remaining);
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// 参数：仅当前事务的 SQL/验证；返回：原错误保持，成功结果须仍在原期限内。
    pub(crate) fn statement<T>(&self, operation: impl FnMut() -> Result<T>) -> Result<T> {
        let value = self.run_sql(operation)?;
        self.check()?;
        Ok(value)
    }

    // 仅重试这两个事务边界，绝不重放含准入/写入副作用的 statement consumer。
    // SQLite 内建 busy 睡眠累计名义间隔，不能替代原绝对时钟；单次系统调用/I/O 仍不可抢占。
    fn run_transaction_boundary(
        &self,
        commit: bool,
        check_execution: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        let sql = if commit { "COMMIT" } else { "BEGIN IMMEDIATE" };
        let host_deadline = Instant::now()
            .checked_add(self.previous_busy)
            .ok_or(StoreError::IntegerOverflow)?;
        self.connection.busy_timeout(Duration::ZERO)?;
        loop {
            check_execution()?;
            self.check()?;
            match self.connection.execute_batch(sql) {
                Ok(()) => return Ok(()),
                Err(error) if matches!(&error, rusqlite::Error::SqliteFailure(code, _) if code.extended_code == rusqlite::ffi::SQLITE_BUSY) =>
                {
                    // 只等待可重试的普通 BUSY；扩展 BUSY_SNAPSHOT、LOCKED 保留原错误。
                    self.check()?;
                    let host_remaining = host_deadline.saturating_duration_since(Instant::now());
                    if host_remaining.is_zero() {
                        return Err(error.into());
                    }
                    let delay = self
                        .remaining()?
                        .min(host_remaining)
                        .min(Duration::from_millis(1));
                    std::thread::sleep(delay);
                    self.check()?;
                    if Instant::now() >= host_deadline {
                        return Err(error.into());
                    }
                }
                Err(error) if matches!(&error, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::OperationInterrupted) =>
                {
                    // 中断不重试；只在原期限/原认证确已失效时保持旧拒绝分类。
                    self.check()?;
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// 参数：无；返回：同一连接的 IMMEDIATE 写事务已取得，失败由守卫清理。
    pub(crate) fn begin(&self) -> Result<()> {
        self.run_transaction_boundary(false, &mut || Ok(()))?;
        self.check()
    }

    /// 参数：原执行检查器；返回：同一原期限内取得事务，BUSY 重试前持续检查取消。
    pub(crate) fn begin_checked(
        &self,
        check_execution: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        self.run_transaction_boundary(false, check_execution)?;
        check_execution()?;
        self.check()
    }

    /// 参数：原执行检查器；返回：原期限和取消门禁下的实际提交，不重放事务内容。
    pub(crate) fn commit_checked(
        &self,
        check_execution: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        self.clear_progress()?;
        self.run_transaction_boundary(true, check_execution)
    }

    fn clear_progress(&self) -> Result<()> {
        if self.progress_installed.get() {
            self.connection.progress_handler(0, None::<fn() -> bool>)?;
            self.progress_installed.set(false);
        }
        Ok(())
    }

    /// 参数：无；返回：实际 COMMIT 结果。调用方先完成最后授权；这里紧贴原期限/原 exp 复检。
    /// COMMIT 的不可抢占 I/O 仍为协作边界，成功事实不由提交后的过期检查伪装成失败。
    pub(crate) fn commit(&self) -> Result<()> {
        // 移除过期 VM 回调后再作纯末检，避免 COMMIT 成功之后的 VM 回调否认已提交事实。
        self.clear_progress()?;
        self.run_transaction_boundary(true, &mut || Ok(()))
    }

    fn cleanup(&self) -> Result<()> {
        if self.finished.get() {
            return Ok(());
        }
        // 即使已有错误，三项均实际尝试；过期 progress 不得阻止 ROLLBACK。
        let cleared = self.clear_progress();
        let rolled_back = if self.connection.is_autocommit() {
            Ok(())
        } else {
            self.connection
                .execute_batch("ROLLBACK")
                .map_err(StoreError::from)
        };
        let restored = self
            .connection
            .busy_timeout(self.previous_busy)
            .map_err(StoreError::from);
        // 清理失败时仍允许 Drop 再尝试；不把尚未还原的连接标记为已完成。
        self.finished
            .set(cleared.is_ok() && rolled_back.is_ok() && restored.is_ok());
        cleared?;
        rolled_back?;
        restored
    }

    /// 参数：事务实际结果；返回：保留原格式/授权/SQL错误，成功路径要求配置清理成功。
    pub(crate) fn finish<T>(self, result: Result<T>) -> Result<T> {
        let cleanup = self.cleanup();
        match result {
            Err(error) => Err(error),
            Ok(value) => {
                cleanup?;
                Ok(value)
            }
        }
    }
}
impl Drop for ControlWriteDeadline<'_> {
    fn drop(&mut self) {
        // Rust unwind 也先清 handler 再回滚；不在 SQLite 回调内 panic/重入数据库。
        let _ = self.cleanup();
    }
}
