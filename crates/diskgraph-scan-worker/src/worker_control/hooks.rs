use std::{
    cell::RefCell,
    io,
    sync::{Mutex, mpsc::Sender},
};

/// 请求局部竞争见证；来源：真实 WorkerControl Mutex/reader，不向槽中注入测试错误。
#[derive(Default)]
struct Hooks {
    after_empty: Option<Box<dyn FnOnce(bool)>>,
    published: Option<Sender<()>>,
    read_returned: Option<Sender<()>>,
}

thread_local! {
    static HOOKS: RefCell<Hooks> = const { RefCell::new(Hooks { after_empty: None, published: None, read_returned: None }) };
}

pub(super) fn install_after_empty(hook: impl FnOnce(bool) + 'static) {
    HOOKS.with(|hooks| hooks.borrow_mut().after_empty = Some(Box::new(hook)));
}

pub(super) fn install_reader_witnesses(returned: Sender<()>, published: Sender<()>) {
    HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        hooks.read_returned = Some(returned);
        hooks.published = Some(published);
    });
}

pub(super) fn take_reader_witnesses() -> (Option<Sender<()>>, Option<Sender<()>>) {
    HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        (hooks.read_returned.take(), hooks.published.take())
    })
}

pub(super) fn after_empty(slot: &Mutex<Option<io::Error>>) {
    let hook = HOOKS.with(|hooks| hooks.borrow_mut().after_empty.take());
    if let Some(hook) = hook {
        // 此时 gate 尚未释放，无 publisher 竞争；仅选择不会自死锁的调度分支，不修改槽内容。
        let lock_available = slot.try_lock().is_ok();
        hook(lock_available);
    }
}
