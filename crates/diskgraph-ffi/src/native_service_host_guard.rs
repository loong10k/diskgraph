use crate::NativeServiceError;
use crate::native_lifecycle::NativeLifecycle;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::JoinHandle;

/// 库内普通函数栈独占的 manager 句柄守卫；公开能力只能借用，不能带走句柄。
/// 来源：DiskGraph 原生 Rust PF-06 scoped owner；正常返回和 unwind 均真实回收。
pub(crate) struct NativeServiceHostGuard {
    lifecycle: Arc<NativeLifecycle>,
    manager: RefCell<Option<JoinHandle<()>>>,
    background_host: PhantomData<Rc<()>>,
}

impl NativeServiceHostGuard {
    /// 仅在已打开数据库、准备队列后启动 manager。参数：lifecycle 为唯一会话状态；返回：宿主 owner。
    pub(crate) fn start(lifecycle: Arc<NativeLifecycle>) -> Result<Self, NativeServiceError> {
        #[cfg(test)]
        let start_hook = crate::native_service_owner_tests::take_manager_start_hook();
        let state = lifecycle.clone();
        let manager = std::thread::Builder::new()
            .name("diskgraph-native-join".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    #[cfg(test)]
                    if let Some(hook) = start_hook {
                        hook();
                    }
                    state.run_manager();
                }));
                if result.is_err() {
                    state.manager_failed();
                }
            })
            .map_err(|error| NativeServiceError::Unavailable {
                reason: format!("coordinator manager spawn failed: {error}"),
            })?;
        Ok(Self {
            lifecycle,
            manager: RefCell::new(Some(manager)),
            background_host: PhantomData,
        })
    }

    /// 在创建此 owner 的非 UI 宿主线程关闭服务并真实 join manager 和异常遗留 worker。
    /// 参数：无；返回：两级协调线程均已回收或原错误，绝不代表 pinned walker 已退出。
    /// 本方法可能等待 TLS/系统调用；不设伪期限，不允许从 UI 调用。
    pub(crate) fn finalize_owner(&self) -> Result<(), NativeServiceError> {
        self.lifecycle.close();
        // 借用只持续到 take，实际 join 不持 RefCell/registry/数据库锁。
        let manager = self.manager.borrow_mut().take();
        if let Some(manager) = manager
            && manager.join().is_err()
        {
            self.lifecycle.manager_failed();
        }
        self.lifecycle.finalize_remaining()
    }
}

impl Drop for NativeServiceHostGuard {
    fn drop(&mut self) {
        let _ = self.finalize_owner();
    }
}
