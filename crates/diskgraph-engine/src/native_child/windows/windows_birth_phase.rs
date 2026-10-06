/// 原 Job 责任的创建阶段。来源：Win32 CreateProcessW 与 PF-06；原生 Rust，无 Java 对等对象。
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum WindowsBirthPhase {
    /// 尚未调用 CreateProcess，确认没有本 owner 的 leader。
    Prepared,
    /// 已进入创建调用，但结果/leader 尚未确认；缺失句柄不能推断未出生。
    Creating,
    /// 成功取得原 process 句柄，必须实际观察 leader 终态。
    ProcessOwned,
    /// 原 CreateProcess 明确返回失败，仍须收取原 Job 与全部 pending I/O。
    CreateFailed,
}
impl WindowsBirthPhase {
    /// 参数：无；返回：本阶段是否已明确确认没有本 owner 的进程出生。
    pub(super) fn confirmed_unborn(self) -> bool {
        matches!(self, Self::Prepared | Self::CreateFailed)
    }
}
