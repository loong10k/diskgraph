use super::linux_supervisor_process::LinuxSupervisorProcess;
use std::io;

/// 保留原启动错误和出生后原进程及容量槽；来源：PF-06 root broker 失败所有权合同。
pub struct LinuxSupervisorLaunchError {
    pub(super) error: io::Error,
    pub(super) owner: Option<Box<LinuxSupervisorProcess>>,
}

impl LinuxSupervisorLaunchError {
    /// 参数：self 为启动失败；返回：原错误和仍须显式恢复的唯一 owner，不清除容量记录。
    pub fn into_parts(self) -> (io::Error, Option<Box<LinuxSupervisorProcess>>) {
        (self.error, self.owner)
    }
}

impl std::fmt::Debug for LinuxSupervisorLaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxSupervisorLaunchError")
            .field("error", &self.error)
            .field("retained_owner", &self.owner.is_some())
            .finish()
    }
}
