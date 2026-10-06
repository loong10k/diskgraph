use std::cell::RefCell;

thread_local! {
    static BEFORE_MARK: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

/// 原生竞态测试的一次性时机控制；来源：Windows删除最后核验/副作用边界，无Java对应。
/// 仅测试构建，回调实际调用文件系统，不模拟链接或系统调用结果。
pub(super) struct WindowsCleanupMarkHook;

impl WindowsCleanupMarkHook {
    /// 参数：action为本线程实际竞态操作；返回：无，拒绝覆盖未消费的回调。
    pub(super) fn install(action: impl FnOnce() + 'static) {
        BEFORE_MARK.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none(), "original before-mark callback still armed");
            *slot = Some(Box::new(action));
        });
    }

    /// 参数：无；返回：无，先取走唯一回调再执行，panic也不会在重试中再次调用。
    pub(super) fn run() {
        let action = BEFORE_MARK.with(|slot| slot.borrow_mut().take());
        if let Some(action) = action {
            action();
        }
    }
}
