/// 当前线程的一次性真实管道描述符观察器。来源：PF-06 原生管道测试夹具，无 Java 对等对象。
type RawPipeHook = Box<dyn FnOnce([i32; 2])>;

#[cfg(test)]
thread_local! {
    static RAW_PIPE_HOOK: std::cell::RefCell<Option<RawPipeHook>> = const { std::cell::RefCell::new(None) };
}
/// 参数：hook 为当前测试线程的一次性原始 FD 观察器；返回：无，仅安装观察器，不取得 FD 所有权。
#[cfg(test)]
pub(in crate::native_child) fn set_raw_pipe_hook(hook: Box<dyn FnOnce([i32; 2])>) {
    RAW_PIPE_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}
#[cfg(test)]
pub(super) fn observe_raw_pipe(channels: [i32; 2]) {
    let hook = RAW_PIPE_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook(channels);
    }
}
