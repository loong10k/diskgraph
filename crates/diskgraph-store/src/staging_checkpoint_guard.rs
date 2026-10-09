//! 暂存真实 commit 的页阈值观察；不在控制 fence 中执行磁盘维护。
use crate::Result;
use rusqlite::{Connection, ffi};
use std::cell::Cell;
use std::ffi::{c_char, c_int, c_void};

/// 原写连接的临时 WAL 回调 owner；来源：DiskGraph 原生暂存与控制锁生命周期。
/// 只观察实际 commit 页数，不拥有事务或缓存授权；连接借用禁止提前关闭原 SQLite。
pub(crate) struct StagingCheckpointGuard<'a> {
    connection: &'a Connection,
    previous: i64,
    pages: Box<Cell<c_int>>,
    hook_installed: bool,
    restored: bool,
}

impl<'a> StagingCheckpointGuard<'a> {
    /// 参数：独占调用者持有的原图库连接；返回：保存阈值并捕获实际 commit 的守卫。
    /// 本连接由 Store 私有持有，未对外提供自定义 WAL hook；不覆盖用户注册回调。
    pub(crate) fn suspend(connection: &'a Connection) -> Result<Self> {
        let previous = connection.query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))?;
        let pages = Box::new(Cell::new(0));
        connection.pragma_update(None, "wal_autocheckpoint", 0)?;
        let mut guard = Self {
            connection,
            previous,
            pages,
            hook_installed: false,
            restored: false,
        };
        // pages 的 Box 地址稳定；Connection 非 Sync，回调与同步提交在同一线程执行。
        // 原连接借用覆盖整个注册期，Drop 在释放 Box 前无条件注销原回调。
        unsafe {
            ffi::sqlite3_wal_hook(
                connection.handle(),
                Some(observe_pages),
                (&*guard.pages as *const Cell<c_int>).cast_mut().cast(),
            );
        }
        guard.hook_installed = true;
        Ok(guard)
    }

    /// 参数：消费本次观察，并累计实际已提交页数达到阈值的请求；返回：配置恢复结果。
    /// 必须先拆除原回调再恢复配置，错误/回滚路径也不能留下悬空回调地址。
    pub(crate) fn finish(mut self, checkpoint_due: &mut bool) -> Result<()> {
        // 恢复配置可能失败；先保留已实际提交的维护责任，不能让 Result 丢掉它。
        *checkpoint_due |= self.previous > 0 && i64::from(self.pages.get()) >= self.previous;
        self.detach();
        self.connection
            .pragma_update(None, "wal_autocheckpoint", self.previous)?;
        self.restored = true;
        Ok(())
    }

    fn detach(&mut self) {
        if self.hook_installed {
            // SQLite 注销回调不执行 SQL，也不会失败；原 Connection 借用仍有效。
            unsafe {
                ffi::sqlite3_wal_hook(self.connection.handle(), None, std::ptr::null_mut());
            }
            self.hook_installed = false;
        }
    }
}

impl Drop for StagingCheckpointGuard<'_> {
    fn drop(&mut self) {
        self.detach();
        // 内层事务先回滚/提交再退出本作用域；恢复失败不在 unwind 中二次 panic。
        if !self.restored
            && self
                .connection
                .pragma_update(None, "wal_autocheckpoint", self.previous)
                .is_err()
        {
            eprintln!("diskgraph: staging automatic checkpoint configuration restore failed");
        }
    }
}

/// SQLite 同步调用的有限观察；只写入仍由守卫持有的 Cell，不分配或调用用户代码。
unsafe extern "C" fn observe_pages(
    context: *mut c_void,
    _database: *mut ffi::sqlite3,
    _name: *const c_char,
    pages: c_int,
) -> c_int {
    // 注册时给出原 Box 内的 Cell；守卫先注销再销毁，且回调不能跨 Connection 线程并发。
    unsafe { &*context.cast::<Cell<c_int>>() }.set(pages);
    ffi::SQLITE_OK
}
