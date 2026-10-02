use std::io;
use std::marker::PhantomData;
use std::rc::Rc;

/// 当前线程的平台占位策略；来源：Apple TN3150 / Windows placeholder compatibility。
/// macOS 禁止物化，Windows 暴露占位属性以供原生打开门禁检查；其他系统无策略。
/// guard 不能跨线程移动，避免在另一线程恢复错误的 I/O 策略。
pub struct HydrationGuard {
    #[cfg(target_os = "macos")]
    previous: i32,
    #[cfg(windows)]
    _windows_mode: crate::windows_placeholder_mode::WindowsPlaceholderMode,
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
        #[cfg(windows)]
        {
            Ok(Self {
                _windows_mode: crate::windows_placeholder_mode::WindowsPlaceholderMode::enter()?,
                thread_bound: PhantomData,
            })
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        Ok(Self {
            thread_bound: PhantomData,
        })
    }
}

#[cfg(target_os = "macos")]
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::HydrationGuard;
    use windows_sys::Wdk::Storage::FileSystem::{
        RtlQueryThreadPlaceholderCompatibilityMode, RtlSetThreadPlaceholderCompatibilityMode,
    };

    #[test]
    fn native_windows_placeholder_mode_is_exposed_and_restored_when_nested() {
        let original = unsafe { RtlSetThreadPlaceholderCompatibilityMode(1) };
        assert!(original >= 0);
        {
            let _outer = HydrationGuard::enter().unwrap();
            assert_eq!(unsafe { RtlQueryThreadPlaceholderCompatibilityMode() }, 2);
            {
                let _inner = HydrationGuard::enter().unwrap();
            }
            assert_eq!(unsafe { RtlQueryThreadPlaceholderCompatibilityMode() }, 2);
        }
        assert_eq!(unsafe { RtlQueryThreadPlaceholderCompatibilityMode() }, 1);
        assert!(unsafe { RtlSetThreadPlaceholderCompatibilityMode(original) } >= 0);
    }
}
