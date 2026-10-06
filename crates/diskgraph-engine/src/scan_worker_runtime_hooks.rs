//! 请求局部真实child出生观察；来源：PF-06实际Engine运行测试，不注入错误或替换算法。

use crate::scan_worker_child::ScanWorkerChild;
use std::cell::RefCell;

type AfterLaunch = Box<dyn FnOnce(&mut ScanWorkerChild)>;

thread_local! {
    static OBSERVER: RefCell<Option<AfterLaunch>> = const { RefCell::new(None) };
}

/// 参数：observer为本线程下一次真实launch的锁外观察；返回：无，不授予执行资格。
#[cfg(any(
    target_os = "linux",
    all(target_os = "macos", feature = "macos_native_scan_candidate")
))]
pub(super) fn at_launch(observer: impl FnOnce(&mut ScanWorkerChild) + 'static) {
    OBSERVER.with(|slot| *slot.borrow_mut() = Some(Box::new(observer)));
}

/// 参数：child是catch外已有唯一owner；返回：无，先取回调再调用，不持TLS借用。
pub(super) fn after_launch(child: &mut ScanWorkerChild) {
    let observer = OBSERVER.with(|slot| slot.borrow_mut().take());
    if let Some(observer) = observer {
        observer(child);
    }
}
