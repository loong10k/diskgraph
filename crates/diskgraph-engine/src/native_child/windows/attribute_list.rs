//! CreateProcessW 扩展属性列表的稳定对齐存储。

use std::ffi::c_void;
use std::io;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{
    DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
    PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, UpdateProcThreadAttribute,
};

use super::super::{ChildError, ChildInputMode};

/// Job、继承句柄与出生前加载策略的对齐所有者。来源：Win32 UpdateProcThreadAttribute 的 lpValue 存活契约。
pub(super) struct AttributeList {
    words: Vec<usize>,
    initialized: bool,
    jobs: Option<Box<[HANDLE; 1]>>,
    handles: Option<Box<[HANDLE; 3]>>,
    mitigation: Box<u64>,
}

impl AttributeList {
    /// 创建出生前属性容器。参数：mode 为原输入用途；返回：属性列表或 Win32 错误。
    pub(super) fn new(mode: ChildInputMode) -> Result<Self, ChildError> {
        let mut bytes = 0usize;
        unsafe { InitializeProcThreadAttributeList(null_mut(), 3, 0, &mut bytes) };
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
            // 扫描用途额外要求 Microsoft 签名加载；普通探针保留原策略。
            // SDK ALWAYS_ON位域：拒绝远程、拒绝低完整性、优先System32；
            // Box在移动AttributeList时保持lpValue地址稳定，存活至Delete之后。
            mitigation: Box::new(
                (1_u64 << 52)
                    | (1_u64 << 56)
                    | (1_u64 << 60)
                    | if mode == ChildInputMode::WorkerControl {
                        1_u64 << 44
                    } else {
                        0
                    },
            ),
        };
        if unsafe { InitializeProcThreadAttributeList(list.as_raw(), 3, 0, &mut bytes) } == 0 {
            // 初始化失败时不能调用 DeleteProcThreadAttributeList。
            let error = ChildError::io(
                "InitializeProcThreadAttributeList",
                io::Error::last_os_error(),
            );
            return Err(error);
        }
        list.initialized = true;
        let policy = (&*list.mitigation as *const u64).cast();
        list.update(
            PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
            policy,
            std::mem::size_of::<u64>(),
        )?;
        // 系统若不支持此准入，原错误直接返回；不得丢弃策略重试出生。
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
