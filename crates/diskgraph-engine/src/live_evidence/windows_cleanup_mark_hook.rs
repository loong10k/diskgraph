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

    /// 参数：file为已成功标记的原DELETE句柄；返回：无，记录真实后置元数据，不改动生产结果。
    pub(super) fn inspect_marked(file: &std::fs::File) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_STANDARD_INFO, FileStandardInfo, GetFileInformationByHandleEx,
        };
        let mut info = FILE_STANDARD_INFO::default();
        let result = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStandardInfo,
                std::ptr::addr_of_mut!(info).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        };
        if result != 0 {
            eprintln!(
                "DG_POST_MARK_STANDARD links={}; pending={}; directory={}",
                info.NumberOfLinks, info.DeletePending, info.Directory
            );
        } else {
            eprintln!(
                "DG_POST_MARK_STANDARD error={:?}",
                std::io::Error::last_os_error().raw_os_error()
            );
        }
    }
}
