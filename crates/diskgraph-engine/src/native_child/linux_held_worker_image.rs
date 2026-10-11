use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::time::Instant;

/// root出生交付的原只读worker镜像；来源：PF-06固定FD合同，无Java对等对象。
/// 本对象只验证原句柄，不自建root出生或摘要来源信任，不打开业务文件路径。
pub(crate) struct LinuxHeldWorkerImage {
    image: File,
}

impl LinuxHeldWorkerImage {
    /// 参数：image为原独占移交File，expected为独立受信预期，deadline为原期限；返回：完整验证或拒绝。
    /// 接收端只能在原root私有出生材料认证后调用，不能采用客户端FD、JSON或镜像自报摘要。
    pub(crate) fn verify(
        mut image: File,
        expected: &ScanWorkerHostConfig,
        deadline: Instant,
    ) -> Result<Self, EngineError> {
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let metadata = image.metadata()?;
        let flags = unsafe { libc::fcntl(image.as_raw_fd(), libc::F_GETFL) };
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.nlink() != 1
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o111 == 0
            || flags < 0
            || flags & libc::O_ACCMODE != libc::O_RDONLY
            || flags & libc::O_PATH != 0
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        let inherited = unsafe { libc::fcntl(image.as_raw_fd(), libc::F_GETFD) };
        if inherited < 0
            || unsafe {
                libc::fcntl(
                    image.as_raw_fd(),
                    libc::F_SETFD,
                    inherited | libc::FD_CLOEXEC,
                )
            } < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        crate::verify_supervisor_birth_image::verify_supervisor_birth_image(
            &mut image, expected, deadline,
        )?;
        Ok(Self { image })
    }

    /// 参数：无；返回：消费验证对象后的原File，不按路径再次打开镜像。
    pub(crate) fn into_file(self) -> File {
        self.image
    }
}
