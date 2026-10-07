use std::cell::UnsafeCell;
use windows_sys::Win32::Foundation::{HANDLE, STATUS_PENDING};
use windows_sys::Win32::System::IO::OVERLAPPED;

/// 唯一稳定OVERLAPPED内存，允许内核异步写入而不创建Rust共享/独占内容引用。
/// 来源：Win32 overlapped I/O与CancelIoEx完成合同；无Java对等对象。
/// 外层管道必须保留event和buffer直到实际完成；本对象不实现Sync。
pub(super) struct WindowsOverlappedOperation {
    storage: Box<UnsafeCell<OVERLAPPED>>,
}

// SAFETY: Box地址在owner移动时不变；没有APC，用户不写入线程局部指针，联合域仅零初始化。
// hEvent属于同进程且由外层唯一管道owner保活。所有操作只由唯一owner串行发起，
// 内核可以写UnsafeCell内存；CancelIoEx/GetOverlappedResult可在接管线程完成。
// 外层在实际完成之前不得reset/drop，本类型不提供内容引用也不提供Sync。
unsafe impl Send for WindowsOverlappedOperation {}

impl WindowsOverlappedOperation {
    /// 参数：由外层保活的event；返回：不会因owner跨线程转移而换址的操作内存。
    pub(super) fn new(event: HANDLE) -> Self {
        let operation = OVERLAPPED {
            hEvent: event,
            ..OVERLAPPED::default()
        };
        Self {
            storage: Box::new(UnsafeCell::new(operation)),
        }
    }
    /// 参数：event 为原 owner 持有的事件句柄，调用前必须确认无 pending；返回：无，原存储地址保持稳定。
    /// 仅外层确认没有pending操作时调用；参数：event沿原owner，不改变内存地址。
    pub(super) fn reset(&mut self, event: HANDLE) {
        let operation = OVERLAPPED {
            hEvent: event,
            ..OVERLAPPED::default()
        };
        unsafe { self.as_ptr().write(operation) };
    }
    /// 参数：无；返回：原 OVERLAPPED 存储地址，借用者不得越过 owner 寿命使用。
    /// 返回：稳定原生地址，不创建指向内核可变字段的Rust引用。
    pub(super) fn as_ptr(&self) -> *mut OVERLAPPED {
        self.storage.get()
    }
    /// 参数：无；返回：原 Internal 字段是否仍为 STATUS_PENDING，不以取消请求成功替代实际完成。
    /// 采用Win32 HasOverlappedIoCompleted同一状态谓词，不以取消返回：代替完成。
    pub(super) fn pending(&self) -> bool {
        unsafe {
            std::ptr::read_volatile(std::ptr::addr_of!((*self.as_ptr()).Internal))
                == STATUS_PENDING as usize
        }
    }
}
