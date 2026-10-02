use std::io;
use std::marker::PhantomData;
use std::rc::Rc;

/// 当前线程的禁止云文件物化策略；与 Apple TN3150 保持一致，离开作用域恢复。
/// guard 不能跨线程移动，避免在另一线程恢复错误的 I/O 策略。
pub struct HydrationGuard {
    #[cfg(target_os = "macos")]
    previous: i32,
    thread_bound: PhantomData<Rc<()>>,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn getiopolicy_np(iotype: i32, scope: i32) -> i32;
    fn setiopolicy_np(iotype: i32, scope: i32, policy: i32) -> i32;
}

impl HydrationGuard {
    /// 在任何路径解析、打开或读取之前设置线程策略；失败时拒绝内容操作。
    pub fn enter() -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        {
            // SDK sys/resource.h：MATERIALIZE_DATALESS_FILES=3, THREAD=1, OFF=1。
            let previous = unsafe { getiopolicy_np(3, 1) };
            if previous < 0 || unsafe { setiopolicy_np(3, 1, 1) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                previous,
                thread_bound: PhantomData,
            })
        }
        #[cfg(not(target_os = "macos"))]
        Ok(Self {
            thread_bound: PhantomData,
        })
    }
}

impl Drop for HydrationGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            // 本类型 !Send，析构发生在设置策略的同一线程。
            unsafe {
                setiopolicy_np(3, 1, self.previous);
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{HydrationGuard, getiopolicy_np, setiopolicy_np};

    #[test]
    fn thread_policy_is_disabled_and_restored_even_when_nested() {
        let original = unsafe { getiopolicy_np(3, 1) };
        assert!(original >= 0);
        assert_eq!(unsafe { setiopolicy_np(3, 1, 2) }, 0);
        {
            let _outer = HydrationGuard::enter().unwrap();
            assert_eq!(unsafe { getiopolicy_np(3, 1) } & 3, 1);
            {
                let _inner = HydrationGuard::enter().unwrap();
            }
            assert_eq!(unsafe { getiopolicy_np(3, 1) } & 3, 1);
        }
        assert_eq!(unsafe { getiopolicy_np(3, 1) } & 3, 2);
        assert_eq!(unsafe { setiopolicy_np(3, 1, original) }, 0);
    }
}
