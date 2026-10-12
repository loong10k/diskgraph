use super::LinuxExecutorPrivacy;
use crate::recovery_control::{LinuxExecutorMaterials, UnixControlWriter};
use crate::recovery_slot::LinuxSupervisorPeer;
use crate::supervisor_service::LinuxExecutorRequest;
use crate::{EngineError, ScanWorkerHost, ScanWorkerRuntimeBudget};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// 用户executor原出生认证及固定镜像采用；来源：PF-06 guardian/executor，无Java对等对象。
/// 不接收容量目录或namespace，不认领槽；原资源只能留在本executor地址空间。
pub struct LinuxExecutorBirth {
    request: LinuxExecutorRequest,
    worker: File,
    control: UnixControlWriter,
    deadline: Instant,
}

impl LinuxExecutorBirth {
    /// 参数：maximum_deadline为原调用启动期限；返回：未配置、认证出生或拒绝，不创建数据库。
    /// 私有FD5须由可信父独占交付；环境本身不认证，必须核对原父逐消息内核凭据和原JSON摘要。
    pub fn receive_from_environment(
        maximum_deadline: Instant,
    ) -> Result<Option<Self>, EngineError> {
        let bytes = std::env::var_os("DISKGRAPH_LINUX_EXECUTOR_BIRTH");
        let fd = std::env::var_os("DISKGRAPH_LINUX_EXECUTOR_FD");
        let bytes = match (bytes, fd) {
            (None, None) => return Ok(None),
            (Some(bytes), Some(fd)) if fd == "5" => bytes,
            _ => return Err(BusinessError::PermissionDenied.into()),
        };
        if std::env::var_os("DISKGRAPH_LINUX_SUPERVISOR_BIRTH").is_some()
            || std::env::var_os("DISKGRAPH_LINUX_SUPERVISOR_FD").is_some()
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        // 在任何Engine、会话或工作子进程初始化前收敛隐私；状态1不能补发安全证明。
        LinuxExecutorPrivacy::enter().map_err(|error| {
            if matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES)) {
                EngineError::from(BusinessError::PermissionDenied)
            } else {
                EngineError::from(error)
            }
        })?;
        let bytes = bytes
            .to_str()
            .ok_or(BusinessError::PermissionDenied)?
            .as_bytes();
        let request = LinuxExecutorRequest::decode(bytes)?;
        let deadline = request.deadline()?.min(maximum_deadline);
        check_deadline(deadline)?;
        let session: [u8; 32] = Sha256::digest(bytes).into();
        let parent_pid = unsafe { libc::getppid() };
        let peer = LinuxSupervisorPeer::from_host(parent_pid, 0, 0)
            .map_err(|_| BusinessError::PermissionDenied)?;
        // 固定出生FD由本入口独占消费；先恢复CLOEXEC，后续认证失败也关闭同一原端点。
        let birth = adopt(5)?;
        let socket = UnixDatagram::from(birth);
        let mut material = LinuxExecutorMaterials::receiver(socket, peer, session, deadline)
            .map_err(|_| BusinessError::PermissionDenied)?;
        let cancel = AtomicBool::new(false);
        loop {
            match material.poll_receive(&cancel) {
                Ok(true) => break,
                Ok(false) => std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(1)),
                ),
                Err(crate::recovery_control::ControlError::Deadline) => {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                Err(_) => return Err(BusinessError::PermissionDenied.into()),
            }
        }
        verify_identity(&request)?;
        check_deadline(deadline)?;
        let worker = File::from(adopt(7)?);
        let control = crate::linux_supervisor_control_admission::adopt_linux_supervisor_control(
            6, 5, parent_pid, session, deadline, true,
        )?;
        let expected = request.worker_expectation()?;
        let worker = super::linux_held_worker_image::LinuxHeldWorkerImage::verify(
            worker, &expected, deadline,
        )?
        .into_file();
        Ok(Some(Self {
            request,
            worker,
            control,
            deadline,
        }))
    }

    /// 参数：无；返回：已认证请求绑定的原数据目录，不提供容量目录或客户文件授权。
    pub fn data_dir(&self) -> &Path {
        self.request.data_dir()
    }

    /// 参数：runtime为原扫描额度，maximum_deadline为原期限上界；返回：原镜像宿主、私有控制与原期限。
    /// 不重新打开worker路径、不认领任何容量槽；调用者须绑定原ExecutorResourceOwner后才发Ready。
    pub fn into_host(
        self,
        runtime: ScanWorkerRuntimeBudget,
        maximum_deadline: Instant,
    ) -> Result<(ScanWorkerHost, UnixControlWriter, Instant), EngineError> {
        let deadline = self.deadline.min(maximum_deadline);
        check_deadline(deadline)?;
        let host = ScanWorkerHost::new_until(
            self.worker,
            self.request.worker_expectation()?,
            runtime,
            deadline,
            &mut || Ok(()),
        )?;
        Ok((host, self.control, deadline))
    }
}

fn adopt(fd: i32) -> Result<OwnedFd, EngineError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(BusinessError::PermissionDenied.into());
    }
    let original = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(original)
}

fn check_deadline(deadline: Instant) -> Result<(), EngineError> {
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}

fn verify_identity(request: &LinuxExecutorRequest) -> Result<(), EngineError> {
    let (expected_uid, expected_gid) = request.identity();
    let (mut real_uid, mut effective_uid, mut saved_uid) = (0_u32, 0_u32, 0_u32);
    let (mut real_gid, mut effective_gid, mut saved_gid) = (0_u32, 0_u32, 0_u32);
    if unsafe { libc::getresuid(&mut real_uid, &mut effective_uid, &mut saved_uid) } != 0
        || unsafe { libc::getresgid(&mut real_gid, &mut effective_gid, &mut saved_gid) } != 0
        || [real_uid, effective_uid, saved_uid] != [expected_uid; 3]
        || [real_gid, effective_gid, saved_gid] != [expected_gid; 3]
    {
        return Err(BusinessError::PermissionDenied.into());
    }
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    let (expected_count, expected_digest) = request.group_binding();
    if count < 0 || count as u32 != expected_count || count > 65536 {
        return Err(BusinessError::PermissionDenied.into());
    }
    let mut groups = vec![0_u32; count as usize];
    if count != 0 && unsafe { libc::getgroups(count, groups.as_mut_ptr()) } != count {
        return Err(BusinessError::PermissionDenied.into());
    }
    // 摘要为完整内核组数组依原顺序的u32 little-endian字节，不查NSS，不截断或补空组。
    let mut digest = Sha256::new();
    for group in groups {
        digest.update(group.to_le_bytes());
    }
    let actual: [u8; 32] = digest.finalize().into();
    if actual != expected_digest {
        return Err(BusinessError::PermissionDenied.into());
    }
    Ok(())
}
