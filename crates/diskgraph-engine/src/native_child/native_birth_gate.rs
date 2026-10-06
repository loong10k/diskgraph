use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Duration;

static BIRTH_GATE: Mutex<()> = Mutex::new(());

/// 协调本进程受控出生与原FD设置CLOEXEC的短窗口。
/// 来源：原生 Rust macOS CLI/MCP 并发描述符合同；无 Java 对等对象。
/// 外部库自行fork/exec不受本门禁约束；持锁期间不得执行数据库或进程等待。
pub(super) struct NativeBirthGate;

impl NativeBirthGate {
    /// 参数：checkpoint借用原取消与绝对期限；返回：短临界区守卫或未经替换的原错误。
    pub(super) fn acquire<E>(
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<MutexGuard<'static, ()>, E> {
        let mut waited = false;
        loop {
            // 等门后每次竞争前复查原请求；不在持门状态调用可能访问数据库的检查点。
            // 无竞争入口保留调用方既有出生前检查次数。
            if waited {
                checkpoint()?;
            }
            match BIRTH_GATE.try_lock() {
                Ok(guard) => return Ok(guard),
                // 这里只保护unit互斥而非业务状态；展开已释放资源，毒化不代表授权可恢复。
                Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
                Err(TryLockError::WouldBlock) => {
                    #[cfg(test)]
                    observe_wait();
                    if !waited {
                        checkpoint()?;
                    }
                    waited = true;
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    static WAIT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
/// 参数：hook 为当前测试线程的一次性等待观察器；返回：无，不取得或释放出生许可。
///
#[cfg(test)]
pub(super) fn set_wait_hook(hook: Box<dyn FnOnce()>) {
    WAIT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(test)]
fn observe_wait() {
    let hook = WAIT_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
