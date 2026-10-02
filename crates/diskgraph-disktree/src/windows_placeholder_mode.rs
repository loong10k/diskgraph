use std::io;
use std::marker::PhantomData;
use std::rc::Rc;

use windows_sys::Win32::Foundation::FARPROC;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

type Setter = unsafe extern "system" fn(i8) -> i8;

/// 本线程的 Windows 占位属性模式；来源：RtlSetThreadPlaceholderCompatibilityMode。
/// 动态解析可选导出，避免缺少 API 的宿主在进程加载阶段失败。
pub(crate) struct WindowsPlaceholderMode {
    previous: i8,
    setter: Setter,
    thread_bound: PhantomData<Rc<()>>,
}

impl WindowsPlaceholderMode {
    /// 解析并设置 EXPOSE；返回同线程 RAII 恢复器，缺 API 时返回 Unsupported。
    pub(crate) fn enter() -> io::Result<Self> {
        let module_name: Vec<u16> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
        // ntdll 是进程核心模块，本类型不加载/卸载模块，使用有效的零结尾名称。
        let module = unsafe { GetModuleHandleW(module_name.as_ptr()) };
        let entry = if module.is_null() {
            None
        } else {
            unsafe {
                GetProcAddress(
                    module,
                    c"RtlSetThreadPlaceholderCompatibilityMode".as_ptr().cast(),
                )
            }
        };
        Self::with_entry(entry)
    }

    fn with_entry(entry: FARPROC) -> io::Result<Self> {
        let entry = entry.ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "placeholder mode unavailable")
        })?;
        // 安全性：只解析指定 ntdll 导出，签名为 NTAPI CHAR(CHAR)，不是任意函数。
        let setter =
            unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, Setter>(entry) };
        let previous = unsafe { setter(2) };
        if previous < 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "placeholder mode unavailable",
            ));
        }
        Ok(Self {
            previous,
            setter,
            thread_bound: PhantomData,
        })
    }
}

impl Drop for WindowsPlaceholderMode {
    fn drop(&mut self) {
        // !Send 保证在原线程恢复；嵌套 guard 按进入前实际模式恢复。
        unsafe { (self.setter)(self.previous) };
    }
}

#[cfg(test)]
mod tests {
    use super::WindowsPlaceholderMode;
    use windows_sys::Wdk::Storage::FileSystem::RtlQueryThreadPlaceholderCompatibilityMode;

    #[test]
    fn a_missing_mode_export_is_unsupported_without_changing_the_thread() {
        let before = unsafe { RtlQueryThreadPlaceholderCompatibilityMode() };
        let error = WindowsPlaceholderMode::with_entry(None)
            .err()
            .expect("missing API must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert_eq!(
            unsafe { RtlQueryThreadPlaceholderCompatibilityMode() },
            before
        );
    }
}
