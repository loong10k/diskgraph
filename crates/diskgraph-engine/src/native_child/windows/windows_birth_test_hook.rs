//! 仅观察真实出生后的原owner；不修改出生或清理结果。
use super::windows_child::WindowsChild;
use std::cell::RefCell;

type Observer = Box<dyn FnMut(&WindowsChild)>;
thread_local! {
    static OBSERVER: RefCell<Option<Observer>> = const { RefCell::new(None) };
}

/// 请求局部出生观察器；来源：PF-06原owner出生恢复验收，无Java对象。
pub(super) struct WindowsBirthTestHook(Option<Observer>);
impl WindowsBirthTestHook {
    /// 安装观察器并保留前值；回调只接收真实原owner借用。
    pub(super) fn install(observer: impl FnMut(&WindowsChild) + 'static) -> Self {
        Self(OBSERVER.with(|slot| slot.replace(Some(Box::new(observer)))))
    }
    /// 原生CreateProcess成功且句柄已归属后调用，不模拟出生。
    pub(super) fn observe(child: &WindowsChild) {
        OBSERVER.with(|slot| {
            if let Some(observer) = slot.borrow_mut().as_mut() {
                observer(child);
            }
        });
    }
}
impl Drop for WindowsBirthTestHook {
    fn drop(&mut self) {
        OBSERVER.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}
