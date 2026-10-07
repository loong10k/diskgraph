use crate::recovery_control::ControlError;

/// 原受信出生层确认的发送进程凭据；来源：Linux SCM_CREDENTIALS / PF-06，无 Java 对等对象。
/// 数字凭据不签发镜像或 PID 寿命信任；调用方必须保留原进程 owner 和出生绑定。
pub struct LinuxSupervisorPeer {
    pid: libc::pid_t,
    uid: libc::uid_t,
    gid: libc::gid_t,
}

impl LinuxSupervisorPeer {
    /// 参数：pid、uid、gid 为原出生的 PID、真实 UID/GID，不从远程消息取得。
    /// 返回：不可变预期凭据；必须由独立可信 launcher 提供，不能用接收消息自认证。
    pub fn from_host(
        pid: libc::pid_t,
        uid: libc::uid_t,
        gid: libc::gid_t,
    ) -> Result<Self, ControlError> {
        if pid <= 0 {
            return Err(ControlError::Protocol);
        }
        Ok(Self { pid, uid, gid })
    }

    /// 比较内核消息凭据与出生时绑定的对端，不据此认证可执行镜像。
    /// 参数：actual 为内核交付的凭据；返回：PID、UID、GID 全部相同才为 true。
    pub(super) fn matches(&self, actual: &libc::ucred) -> bool {
        self.pid == actual.pid && self.uid == actual.uid && self.gid == actual.gid
    }
}
