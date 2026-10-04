use crate::NativeServiceError;
use crate::native_service_host_guard::NativeServiceHostGuard;
use std::marker::PhantomData;
use std::rc::Rc;

/// 非 UI 宿主作用域内的借用能力，不拥有或导出 manager 句柄。
/// 来源：DiskGraph 原生 Rust PF-06；私有不变 brand 禁止跨作用域交换及静态 TLS 逃逸。
pub struct NativeServiceOwner<'host> {
    guard: &'host NativeServiceHostGuard,
    invariant_host: PhantomData<&'host mut &'host ()>,
    background_host: PhantomData<Rc<()>>,
}

impl<'host> NativeServiceOwner<'host> {
    /// 借用库内唯一栈守卫。参数：guard 为本次宿主作用域的实际所有者；返回：不可逃逸的能力。
    pub(crate) fn borrow(guard: &'host NativeServiceHostGuard) -> Self {
        Self {
            guard,
            invariant_host: PhantomData,
            background_host: PhantomData,
        }
    }

    /// 在后台宿主作用域中关闭准入，真实 join manager 和异常遗留 worker；重复调用幂等。
    /// 参数：无；返回：两级协调线程实际回收结果，不代表 pinned walker 退出。
    /// 本方法可能等待 TLS/系统调用，不支持在 TLS 析构、DllMain 或 UI 上下文调用。
    pub fn finalize_owner(&mut self) -> Result<(), NativeServiceError> {
        self.guard.finalize_owner()
    }
}

impl Drop for NativeServiceOwner<'_> {
    fn drop(&mut self) {
        // 提前释放能力仍执行真实 finalization；forget 不会遗弃独立库栈上的 guard。
        let _ = self.finalize_owner();
    }
}
