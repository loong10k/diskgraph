use super::ControlError;
use std::os::fd::RawFd;

/// 原Unix连接的内核UID/GID及辅助组快照；来源：Linux SO_PEERCRED/SO_PEERGROUPS，无Java对等对象。
/// 不读取客户声明、账户组数据库或PID路径；不证明LSM上下文、镜像或进程寿命。
pub struct LinuxPeerUnixIdentity {
    peer: libc::ucred,
    groups: Vec<u32>,
}

impl LinuxPeerUnixIdentity {
    /// 参数：fd为原连接，expected_uid为可信准入UID；返回：内核Unix身份或拒绝，不降级为空组。
    pub(crate) fn capture(fd: RawFd, expected_uid: u32) -> Result<Self, ControlError> {
        if expected_uid == 0 || expected_uid == u32::MAX {
            return Err(ControlError::Protocol);
        }
        let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of_val(&peer) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut peer as *mut libc::ucred).cast(),
                &mut size,
            )
        } != 0
            || size as usize != std::mem::size_of_val(&peer)
            || peer.pid <= 0
            || peer.uid != expected_uid
            || peer.gid == u32::MAX
        {
            return Err(ControlError::Protocol);
        }
        // SO_PEERGROUPS读取连接时同一sk_peer_cred；不会混入PID复用或后来修改的账户组。
        // 先问原长度，只分配实际组数组；内核不支持、超Linux上限或长度异常均拒绝。
        size = 0;
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERGROUPS,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ERANGE) {
            return Err(ControlError::Unconfirmed);
        }
        let width = std::mem::size_of::<u32>();
        if size as usize > 65536 * width || !(size as usize).is_multiple_of(width) {
            return Err(ControlError::Protocol);
        }
        let mut groups = vec![0_u32; size as usize / width];
        if !groups.is_empty() {
            let expected_size = size;
            if unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_PEERGROUPS,
                    groups.as_mut_ptr().cast(),
                    &mut size,
                )
            } != 0
                || size != expected_size
                || groups.contains(&u32::MAX)
            {
                return Err(ControlError::Unconfirmed);
            }
        }
        Ok(Self { peer, groups })
    }

    /// 参数：无；返回：原连接创建者UID，不代替逐消息发送者或远程token认证。
    pub fn uid(&self) -> u32 {
        self.peer.uid
    }

    /// 参数：无；返回：原连接创建者有效GID，不从安装配置或请求正文读取。
    pub fn gid(&self) -> u32 {
        self.peer.gid
    }

    /// 参数：无；返回：内核保存的完整辅助组借用，不允许请求方修改身份快照。
    pub fn supplementary_groups(&self) -> &[u32] {
        &self.groups
    }

    /// 参数：无；返回：原连接创建者PID，不证明其寿命或当前执行镜像。
    pub(crate) fn pid(&self) -> libc::pid_t {
        self.peer.pid
    }
}
