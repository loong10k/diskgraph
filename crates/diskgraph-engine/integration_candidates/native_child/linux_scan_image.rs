use crate::scan_image_identity::ScanImageIdentity;
use crate::scan_worker_host_config::MAX_IMAGE_BYTES;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::time::Instant;

/// 唯一拥有已核验并由内核密封的执行字节副本，不提供执行或发布许可。
/// 来源：原生 Rust PF-06、Linux memfd_create 与 fcntl 文件密封合同。
pub(crate) struct LinuxScanImage {
    file: File,
}

impl LinuxScanImage {
    /// 参数：source 为原已打开普通文件，config 为独立可信预期值，deadline 为原绝对期限，
    /// checkpoint 保留原授权与取消错误。返回：真实密封的同批核验字节，或原错误。
    /// 不重新按路径定位，不使用邻接清单，不把源文件核验与复制分为两个读取窗口。
    pub(crate) fn prepare(
        mut source: File,
        config: &ScanWorkerHostConfig,
        deadline: Instant,
        mut checkpoint: impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        check(deadline, &mut checkpoint)?;
        let identity = ScanImageIdentity::capture(&source)?;
        let length = source.metadata()?.len();
        if length > MAX_IMAGE_BYTES {
            return Err(BusinessError::BudgetExceeded.into());
        }
        if length != config.expected_bytes {
            return Err(BusinessError::Conflict.into());
        }
        check(deadline, &mut checkpoint)?;
        source.seek(SeekFrom::Start(0))?;
        // 安全性：固定NUL结尾名称只用于匿名对象诊断；不依赖文件路径或宿主环境。
        let descriptor = unsafe {
            libc::memfd_create(
                c"diskgraph-scan-image".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // 安全性：成功创建的描述符只在此处转为唯一File，所有错误与unwind由RAII关闭。
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        let mut block = [0_u8; 64 * 1024];
        let mut remaining = config.expected_bytes;
        let mut digest = Sha256::new();
        while remaining != 0 {
            check(deadline, &mut checkpoint)?;
            let admitted = remaining.min(block.len() as u64) as usize;
            let read = match source.read(&mut block[..admitted]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if read == 0 {
                return Err(BusinessError::Conflict.into());
            }
            let mut written = 0;
            while written < read {
                check(deadline, &mut checkpoint)?;
                let count = match file.write(&block[written..read]) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    result => result?,
                };
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "sealed image copy made no progress",
                    )
                    .into());
                }
                written += count;
            }
            // 摘要只覆盖实际写入副本的原缓冲，不再次读取可变源文件。
            digest.update(&block[..read]);
            remaining -= read as u64;
        }
        let actual: [u8; 32] = digest.finalize().into();
        if actual != config.expected_sha256 {
            return Err(BusinessError::Conflict.into());
        }
        check(deadline, &mut checkpoint)?;
        let required =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        // 安全性：唯一memfd仍存活，固定整型seals，不传指针且不创建可写映射。
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, required) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let observed = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
        if observed < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if observed & required != required {
            return Err(BusinessError::Conflict.into());
        }
        // 最后外部回调在源身份末检之前；sealed副本已不允许内容写入/扩缩。
        check(deadline, &mut checkpoint)?;
        if ScanImageIdentity::capture(&source)? != identity {
            return Err(BusinessError::Conflict.into());
        }
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(Self { file })
    }

    /// 参数：消费唯一密封材料；返回：原memfd File供原子启动器按描述符执行。
    /// 不重开路径或复制owner；ELF格式、装载环境、原子出生与实际退出仍须独立验证。
    pub(crate) fn into_file(self) -> File {
        self.file
    }
}

fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    checkpoint()?;
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}
