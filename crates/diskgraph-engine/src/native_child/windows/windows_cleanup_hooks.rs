//! 仅测试线程局部的一次性cleanup失败；不改变生产算法，不把注入称为内核拒绝。
use std::cell::Cell;
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, SetLastError, WAIT_FAILED};

thread_local! {
    // 注入阶段、真实wait次数、真实accounting次数、实际消费注入次数。
    static STATE: Cell<(u8, u32, u32, u32)> = const { Cell::new((0, 0, 0, 0)) };
}

/// Windows恢复回归的请求局部观察器；来源：PF-06原owner失败重试合同，无Java对象。
pub(super) struct WindowsCleanupHooks;
impl WindowsCleanupHooks {
    /// 参数：stage为1=wait/2=query；重置计数并仅注入一次。
    pub(super) fn arm(stage: u8) {
        assert!(stage == 1 || stage == 2);
        STATE.with(|state| state.set((stage, 0, 0, 0)));
    }
    /// 卸除尚未消费的注入；保留真实调用计数供finally之后断言。
    pub(super) fn disarm() {
        STATE.with(|state| {
            let (_, wait, query, injected) = state.get();
            state.set((0, wait, query, injected));
        });
    }
    /// 返回真实wait/query和已注入次数；不计独立救援句柄的调用。
    pub(super) fn counts() -> (u32, u32, u32) {
        STATE.with(|state| {
            let (_, wait, query, injected) = state.get();
            (wait, query, injected)
        })
    }
    /// 在原wait位置一次性返回WAIT_FAILED；随后原函数真实调用且计数。
    pub(super) fn wait(native: impl FnOnce() -> u32) -> u32 {
        if Self::observe(1) {
            unsafe { SetLastError(ERROR_ACCESS_DENIED) };
            WAIT_FAILED
        } else {
            native()
        }
    }
    /// 在原Job查询位置一次性返回失败；不伪造accounting内容或成功结果。
    pub(super) fn query(native: impl FnOnce() -> i32) -> i32 {
        if Self::observe(2) {
            unsafe { SetLastError(ERROR_ACCESS_DENIED) };
            0
        } else {
            native()
        }
    }
    fn observe(stage: u8) -> bool {
        STATE.with(|state| {
            let (armed, mut wait, mut query, mut injected) = state.get();
            if armed == stage {
                injected += 1;
                state.set((0, wait, query, injected));
                true
            } else {
                if stage == 1 {
                    wait += 1;
                } else {
                    query += 1;
                }
                state.set((armed, wait, query, injected));
                false
            }
        })
    }
}
