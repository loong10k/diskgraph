//! 请求局部启动见证；只观察真实句柄，不注入 pending 或替代被测 owner 清理。

use std::cell::RefCell;
use windows_sys::Win32::Foundation::HANDLE;

use super::super::ChildError;
use super::windows_control_spawn_record::WindowsControlSpawnRecord;

thread_local! {
    static SPAWN: RefCell<Option<WindowsControlSpawnRecord>> = const { RefCell::new(None) };
}

/// 同调用线程的启动观察开关与恢复 guard；来源：原生 Rust 请求局部测试与 Win32 启动见证。
pub(super) struct WindowsControlTestWitness {
    previous: Option<WindowsControlSpawnRecord>,
}

impl WindowsControlTestWitness {
    /// 开启本请求的真实启动观察，不改变创建流程。参数：无。返回：恢复先前配置的局部 guard。
    pub(super) fn enable() -> Self {
        Self {
            previous: SPAWN.with(|slot| slot.replace(Some(WindowsControlSpawnRecord::new()))),
        }
    }

    /// 仅在真实 Job 与 leader owner 安装后记录。参数：job、leader 为仍存活的原句柄。返回：无；复制错误仅留给测试断言。
    pub(super) fn record(job: HANDLE, leader: HANDLE) {
        SPAWN.with(|slot| {
            if let Some(record) = slot.borrow_mut().as_mut() {
                record.record(job, leader);
            }
        });
    }

    /// 返回实际创建次数。参数：无。返回：本请求记录到的 Job／leader 对数。
    pub(super) fn creations(&self) -> usize {
        SPAWN.with(|slot| {
            slot.borrow()
                .as_ref()
                .map_or(0, WindowsControlSpawnRecord::creations)
        })
    }

    /// 持复制句柄观察 owner 清理后的事实，不靠关闭见证句柄完成测试。参数：无。返回：leader 已退出且 Job 活动数为零，或真实查询错误。
    pub(super) fn terminated_and_empty(&self) -> Result<bool, ChildError> {
        SPAWN.with(|slot| {
            slot.borrow()
                .as_ref()
                .ok_or(ChildError::Unsupported("no actual spawn witness"))?
                .terminated_and_empty()
        })
    }
}

impl Drop for WindowsControlTestWitness {
    fn drop(&mut self) {
        SPAWN.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}
