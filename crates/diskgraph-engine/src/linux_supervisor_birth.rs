use crate::linux_supervisor_birth_request::LinuxSupervisorBirthRequest;
use crate::recovery_slot::{
    LinuxSupervisorMaterials, LinuxSupervisorNamespace, LinuxSupervisorPeer,
};
use crate::{EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRuntimeBudget};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// root 原出生通道认证后的 Linux doctor 监督材料；来源 PF-06，无 Java 对等对象。
/// 原命名空间与期限保持到真实退休；不从邻接清单、远程参数或 inode 数字签发信任。
pub struct LinuxSupervisorBirth {
    request: LinuxSupervisorBirthRequest,
    namespace: LinuxSupervisorNamespace,
    deadline: Instant,
}

impl LinuxSupervisorBirth {
    /// 参数：无，读取出生层的两个专用环境值；返回：未配置、认证材料或原拒绝。
    /// 只能在进程启动时调用一次；FD 由出生层独占移交，本函数消费并关闭它。
    /// 环境值本身不可信；原 JSON 的 SHA 必须匹配原父 root 进程内核凭据认证的交付。
    pub fn receive_from_environment() -> Result<Option<Self>, EngineError> {
        let bytes = std::env::var_os("DISKGRAPH_LINUX_SUPERVISOR_BIRTH");
        let descriptor = std::env::var_os("DISKGRAPH_LINUX_SUPERVISOR_FD");
        let (bytes, descriptor) = match (bytes, descriptor) {
            (None, None) => return Ok(None),
            (Some(bytes), Some(descriptor)) => (bytes, descriptor),
            _ => return Err(BusinessError::PermissionDenied.into()),
        };
        let bytes = bytes
            .to_str()
            .ok_or(BusinessError::PermissionDenied)?
            .as_bytes();
        let request = LinuxSupervisorBirthRequest::parse(bytes)?;
        let uid = unsafe { libc::geteuid() };
        if uid != request.service_uid || unsafe { libc::getuid() } != uid {
            return Err(BusinessError::PermissionDenied.into());
        }
        let deadline = request.deadline.adopt(Duration::from_secs(30))?;
        let text = descriptor.to_str().ok_or(BusinessError::PermissionDenied)?;
        if text.is_empty() || text.len() > 10 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(BusinessError::PermissionDenied.into());
        }
        let fd: libc::c_int = text.parse().map_err(|_| BusinessError::PermissionDenied)?;
        if fd < 3 || unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
            return Err(BusinessError::PermissionDenied.into());
        }
        // 在采用句柄前核对真实内核类型，拒绝普通文件、公开 stdio 或 stream 描述符。
        let mut kind: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut kind as *mut libc::c_int).cast(),
                &mut length,
            )
        } != 0
            || kind != libc::SOCK_DGRAM
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        let mut domain: libc::c_int = 0;
        length = std::mem::size_of_val(&domain) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_DOMAIN,
                (&mut domain as *mut libc::c_int).cast(),
                &mut length,
            )
        } != 0
            || domain != libc::AF_UNIX
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        // 出生层独占移交；拒绝也不遗留材料通道。后续 new 再核对未命名已连接 AF_UNIX。
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        // 显式恢复继承通道的 close-on-exec，不能让后续 worker 继承 root 出生端点。
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(BusinessError::PermissionDenied.into());
        }
        let socket = UnixDatagram::from(owned);
        let session: [u8; 32] = Sha256::digest(bytes).into();
        let peer = LinuxSupervisorPeer::from_host(unsafe { libc::getppid() }, 0, 0)
            .map_err(|_| BusinessError::PermissionDenied)?;
        let mut materials = LinuxSupervisorMaterials::new(socket, peer, session, deadline)
            .map_err(|_| BusinessError::PermissionDenied)?;
        let trust = materials.receive(&AtomicBool::new(false)).map_err(|_| {
            if Instant::now() >= deadline {
                BusinessError::BudgetExceeded
            } else {
                BusinessError::PermissionDenied
            }
        })?;
        // 接收耗时从原 ClockStamp 再次扣除，不能由角色启动重新获得完整 30 秒。
        let deadline = request.deadline.adopt(Duration::from_secs(30))?;
        let namespace = LinuxSupervisorNamespace::from_host(
            &request.state_root,
            trust,
            request.frontend_uid,
            deadline,
        )?;
        Ok(Some(Self {
            request,
            namespace,
            deadline,
        }))
    }

    /// 参数：无；返回：root 出生材料绑定的数据目录，调用方不得用客户端定位替换。
    pub fn data_dir(&self) -> &Path {
        &self.request.data_dir
    }

    /// 参数：无；返回：原启动期限，不因 Engine 构造或恢复角色切换刷新。
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// 参数：runtime 为原服务扫描额度，maximum_deadline 为调用方原期限；返回：已核验的原固定镜像及保持原域寿命的材料。
    /// 此调用尚不出生原生工作；失败不能启用普通本地兼容回退。
    pub fn into_host(
        mut self,
        runtime: ScanWorkerRuntimeBudget,
        maximum_deadline: Instant,
    ) -> Result<(ScanWorkerHost, LinuxSupervisorNamespace, Instant), EngineError> {
        self.deadline = self.deadline.min(maximum_deadline);
        if Instant::now() >= self.deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let digest: [u8; 32] = hex::decode(&self.request.worker_sha256)
            .map_err(|_| BusinessError::PermissionDenied)?
            .try_into()
            .map_err(|_| BusinessError::PermissionDenied)?;
        let expected =
            ScanWorkerHostConfig::from_expected_image(digest, self.request.worker_bytes)?;
        let image = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.request.worker_path)?;
        let host =
            ScanWorkerHost::new_until(image, expected, runtime, self.deadline, &mut || Ok(()))?;
        Ok((host, self.namespace, self.deadline))
    }
}
