use std::cell::RefCell;
#[cfg(windows)]
use std::marker::PhantomData;
#[cfg(windows)]
use std::rc::Rc;

thread_local! {
    static AFTER_COMMIT: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

/// 请求线程局部的真实提交后观察点；来源：原生 Rust D45 撤权通知顺序回归。
/// 仅测试构建存在，不替换事务结果，也不持 registry 或 SQLite 锁调用测试闭包。
#[cfg(windows)]
pub(crate) struct WithdrawalPublishHook {
    _same_thread: PhantomData<Rc<()>>,
}

#[cfg(windows)]
impl WithdrawalPublishHook {
    /// 安装一次真实提交成功之后、负向通知之前的观察闭包。
    /// 参数：callback 为当前线程独占的一次性观察；返回：清理未消费观察点的守卫。
    pub(crate) fn install(callback: impl FnOnce() + 'static) -> Self {
        AFTER_COMMIT.with(|slot| {
            assert!(slot.borrow().is_none(), "withdrawal hook already installed");
            *slot.borrow_mut() = Some(Box::new(callback));
        });
        Self {
            _same_thread: PhantomData,
        }
    }
}

#[cfg(windows)]
impl Drop for WithdrawalPublishHook {
    fn drop(&mut self) {
        AFTER_COMMIT.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

/// 消费当前线程观察点；必须仅在实际 tx.commit 成功之后调用。
/// 参数：无；返回：无；闭包在 RefCell 借用释放后执行，不伪造 generation 或提交事实。
pub(crate) fn after_commit() {
    let callback = AFTER_COMMIT.with(|slot| slot.borrow_mut().take());
    if let Some(callback) = callback {
        callback();
    }
}
