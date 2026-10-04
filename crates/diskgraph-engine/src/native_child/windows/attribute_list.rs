//! CreateProcessW 扩展属性列表的稳定对齐存储。

use std::ffi::c_void;
use std::io;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{
    DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, UpdateProcThreadAttribute,
};

use super::super::ChildError;

/// 两个扩展属性及其值的对齐所有者。来源：Win32 UpdateProcThreadAttribute 的 lpValue 存活契约。
pub(super) struct AttributeList {
    words: Vec<usize>,
    initialized: bool,
    jobs: Option<Box<[HANDLE; 1]>>,
    handles: Option<Box<[HANDLE; 3]>>,
}

impl AttributeList {
    /// 创建 JOB_LIST 和 HANDLE_LIST 容器。参数：无。返回：已初始化的属性列表或 Win32 错误。
    pub(super) fn new() -> Result<Self, ChildError> {
        let mut bytes = 0usize;
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 || bytes > 1 << 20 {
            return Err(ChildError::io(
                "InitializeProcThreadAttributeList(size)",
                io::Error::last_os_error(),
            ));
        }
        let words = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
        let mut list = Self {
            words,
            initialized: false,
            jobs: None,
            handles: None,
        };
        if unsafe { InitializeProcThreadAttributeList(list.as_raw(), 2, 0, &mut bytes) } == 0 {
            // 初始化失败时不能调用 DeleteProcThreadAttributeList。
            let error = ChildError::io(
                "InitializeProcThreadAttributeList",
                io::Error::last_os_error(),
            );
            return Err(error);
        }
        list.initialized = true;
        Ok(list)
    }

    /// 在创建时绑定本次 Job，属性值持有到 DeleteProcThreadAttributeList 后。
    /// 参数：jobs 为仍存活的 Job 句柄数组，只允许设置一次。返回：设置成功或错误。
    pub(super) fn set_jobs(&mut self, jobs: [HANDLE; 1]) -> Result<(), ChildError> {
        if self.jobs.is_some() {
            return Err(ChildError::Unsupported("probe Job attribute already set"));
        }
        self.jobs = Some(Box::new(jobs));
        let value = self
            .jobs
            .as_ref()
            .expect("just installed Job attribute")
            .as_ptr()
            .cast();
        self.update(
            PROC_THREAD_ATTRIBUTE_JOB_LIST,
            value,
            std::mem::size_of::<[HANDLE; 1]>(),
        )
    }

    /// 限定子进程可继承的标准句柄，属性值持有到 DeleteProcThreadAttributeList 后。
    /// 参数：handles 为三个仍存活的标准句柄数组，只允许设置一次。返回：设置成功或错误。
    pub(super) fn set_handles(&mut self, handles: [HANDLE; 3]) -> Result<(), ChildError> {
        if self.handles.is_some() {
            return Err(ChildError::Unsupported(
                "probe handle attribute already set",
            ));
        }
        self.handles = Some(Box::new(handles));
        let value = self
            .handles
            .as_ref()
            .expect("just installed handle attribute")
            .as_ptr()
            .cast();
        self.update(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            value,
            std::mem::size_of::<[HANDLE; 3]>(),
        )
    }

    /// 借用原始属性列表。参数：无。返回：CreateProcessW 使用的稳定指针。
    pub(super) fn as_raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.words.as_mut_ptr().cast()
    }

    fn update(&mut self, kind: u32, value: *const c_void, bytes: usize) -> Result<(), ChildError> {
        if unsafe {
            UpdateProcThreadAttribute(
                self.as_raw(),
                0,
                kind as usize,
                value,
                bytes,
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(ChildError::io(
                "UpdateProcThreadAttribute",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        if self.initialized {
            // Windows 会借用 lpValue 至属性列表销毁；随后才由字段自动释放 Box。
            unsafe { DeleteProcThreadAttributeList(self.as_raw()) };
        }
    }
}
