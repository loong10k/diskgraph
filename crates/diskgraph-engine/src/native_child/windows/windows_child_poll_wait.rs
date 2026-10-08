//! 有限等待原重叠 I/O 的完成事件；唤醒不授予读取、协议完成或正常退出许可。

use super::WindowsChild;
use crate::native_child::ChildError;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::WaitForMultipleObjects;

impl WindowsChild {
    /// 等待原 pending I/O 事件或有限退避。参数：deadline 为原请求期限；返回：唤醒或原 Win32 错误。
    /// 借用期间原 owner 不释放句柄，唤醒后仍由下一轮 poll 完成全部安全检查。
    pub(crate) fn wait_for_io_until(&self, deadline: Instant) -> Result<(), ChildError> {
        let mut events = [std::ptr::null_mut(); 3];
        let mut count = 0;
        for event in [
            self.stdout.as_ref().and_then(|pipe| pipe.pending_event()),
            self.stderr.as_ref().and_then(|pipe| pipe.pending_event()),
            self.control.as_ref().and_then(|pipe| pipe.pending_event()),
        ]
        .into_iter()
        .flatten()
        {
            events[count] = event;
            count += 1;
        }
        wait_events_until(&events[..count], deadline)
    }
}

fn wait_events_until(events: &[HANDLE], deadline: Instant) -> Result<(), ChildError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(());
    }
    let delay = remaining.min(Duration::from_millis(20));
    if events.is_empty() {
        std::thread::sleep(delay);
        return Ok(());
    }
    // 向下取整避免扩大原剩余期限；不足1ms时立即回到原检查，不使用INFINITE。
    let milliseconds = delay.as_millis() as u32;
    let status =
        unsafe { WaitForMultipleObjects(events.len() as u32, events.as_ptr(), 0, milliseconds) };
    if status == WAIT_TIMEOUT
        || (WAIT_OBJECT_0..WAIT_OBJECT_0 + events.len() as u32).contains(&status)
    {
        Ok(())
    } else {
        Err(ChildError::io(
            "WaitForMultipleObjects(scan I/O)",
            std::io::Error::last_os_error(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::wait_events_until;
    use crate::native_child::windows::owned_handle::OwnedHandle;
    use std::ptr::null;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    #[test]
    fn ready_event_wakes_without_consuming_the_original_manual_event() {
        let event = OwnedHandle::from_raw(
            unsafe { CreateEventW(null(), 1, 1, null()) },
            "CreateEventW(poll wait test)",
        )
        .unwrap();
        wait_events_until(&[event.as_raw()], Instant::now() + Duration::from_secs(5)).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(event.as_raw(), 0) },
            WAIT_OBJECT_0
        );
    }

    #[test]
    fn expired_wait_never_touches_invalid_native_handles() {
        wait_events_until(&[std::ptr::null_mut()], Instant::now()).unwrap();
    }

    #[test]
    fn unsignaled_event_stays_pending_after_bounded_wait() {
        let event = OwnedHandle::from_raw(
            unsafe { CreateEventW(null(), 1, 0, null()) },
            "CreateEventW(poll timeout test)",
        )
        .unwrap();
        wait_events_until(&[event.as_raw()], Instant::now() + Duration::from_millis(1)).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(event.as_raw(), 0) },
            WAIT_TIMEOUT
        );
    }

    #[test]
    fn live_invalid_handle_preserves_wait_failure() {
        assert!(
            wait_events_until(
                &[std::ptr::null_mut()],
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
