use crate::scan_image_identity::ScanImageIdentity;
use crate::scan_worker_host_config::MAX_IMAGE_BYTES;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

/// 唯一拥有原已打开镜像的安装核验材料；不提供启动、扫描或发布许可。
/// 来源：原生 Rust PF-06 受信宿主安装材料合同；无 Java 对等对象。
#[derive(Debug)]
pub struct ScanWorkerInstallation {
    file: File,
    sha256: [u8; 32],
    bytes: u64,
}

impl ScanWorkerInstallation {
    /// 参数：file 为宿主已打开的普通镜像，config 为独立预期值，deadline 为原绝对期限，
    /// checkpoint 为原授权/取消检查。返回：唯一原 File 与完整核验材料，或原类型错误。
    /// 验证至执行之间的内容稳定与实际镜像绑定，仍须由原生启动机制单独保证。
    pub fn verify(
        mut file: File,
        config: &ScanWorkerHostConfig,
        original_deadline: Instant,
        mut checkpoint: impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        check(original_deadline, &mut checkpoint)?;
        let before = ScanImageIdentity::capture(&file)?;
        let metadata = file.metadata()?;
        if metadata.len() > MAX_IMAGE_BYTES {
            return Err(BusinessError::BudgetExceeded.into());
        }
        if metadata.len() != config.expected_bytes {
            return Err(BusinessError::Conflict.into());
        }
        check(original_deadline, &mut checkpoint)?;
        file.seek(SeekFrom::Start(0))?;
        let mut remaining = config.expected_bytes;
        let mut block = [0_u8; 64 * 1024];
        let mut digest = Sha256::new();
        while remaining != 0 {
            check(original_deadline, &mut checkpoint)?;
            let admitted = remaining.min(block.len() as u64) as usize;
            let read = file.read(&mut block[..admitted])?;
            if read == 0 {
                return Err(BusinessError::Conflict.into());
            }
            digest.update(&block[..read]);
            remaining -= read as u64;
        }
        // 最后一次外部检查位于末次句柄观察之前；任意回调不能在末检之后重定位/修改镜像。
        check(original_deadline, &mut checkpoint)?;
        if ScanImageIdentity::capture(&file)? != before {
            return Err(BusinessError::Conflict.into());
        }
        check_time(original_deadline)?;
        let sha256: [u8; 32] = digest.finalize().into();
        if sha256 != config.expected_sha256 {
            return Err(BusinessError::Conflict.into());
        }
        Ok(Self {
            file,
            sha256,
            bytes: config.expected_bytes,
        })
    }

    /// 参数：无；返回：已核验的完整镜像 SHA-256，不代表平台执行或签名许可。
    pub fn image_sha256(&self) -> [u8; 32] {
        self.sha256
    }

    /// 参数：无；返回：独立预期与原句柄一致的完整镜像长度。
    pub fn image_bytes(&self) -> u64 {
        self.bytes
    }

    /// 参数：消费唯一材料；返回：原 File 原样转移，不按路径重新打开或克隆。
    /// 原生调用方还须约束验证后的内容变化与实际 exec 镜像；此方法不启动进程。
    pub fn into_file(self) -> File {
        self.file
    }
}

fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    // 优先返回原检查点对象，不克隆或由显示文本恢复权限/取消原因。
    checkpoint()?;
    check_time(deadline)
}

fn check_time(deadline: Instant) -> Result<(), EngineError> {
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}
