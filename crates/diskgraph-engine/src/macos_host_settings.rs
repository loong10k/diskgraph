use crate::macos_installation_trust::MacosInstallationTrust;
use crate::macos_protected_document::MacosProtectedDocument;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Instant;

const ACTIVE_CONFIGURATION: &[u8] =
    b"/Library/Application Support/DiskGraph/scan-worker/active.json";

/// 固定root保护位置的活跃安装材料；receipt及普通环境不能提供本对象的独立信任。
/// 来源：原生 Rust PF-06 可信宿主配置合同；无 Java 对等对象。
/// epoch_floor由可信更新者持久发布，本对象不声明可抵抗root同时回滚全部材料。
#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct MacosHostSettings {
    schema_version: u32,
    pub(super) public_key: [u8; 32],
    pub(super) active_epoch: u64,
    epoch_floor: u64,
    pub(super) expected_sha256: [u8; 32],
    pub(super) expected_bytes: u64,
    pub(super) installation_root: Vec<u8>,
    pub(super) receipt_path: Vec<u8>,
}

impl MacosHostSettings {
    /// 参数：无；返回：所有活跃信任字段的规范化域隔离摘要，保留原生路径字节。
    pub(super) fn binding_digest(&self) -> Result<[u8; 32], EngineError> {
        let bytes = serde_json::to_vec(self).map_err(|_| BusinessError::InvalidArgument)?;
        let mut hash = Sha256::new();
        hash.update(b"diskgraph.macos.active-settings\0v1\0");
        hash.update(bytes);
        Ok(hash.finalize().into())
    }

    /// 参数：原安装共享锁与原期限；返回：同时通过独立版本下限验证的配置。
    pub(super) fn read_authorized(
        guard: &crate::macos_installation_lock::MacosInstallationLock,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        let floor = crate::macos_epoch_floor::MacosEpochFloor::read(guard, deadline, checkpoint)?;
        let settings = Self::read_active(deadline, checkpoint)?.ok_or(BusinessError::Conflict)?;
        floor.verify(&settings)?;
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(settings)
    }

    /// 参数：deadline/checkpoint沿原请求；返回：固定保护文件的材料或缺失状态。
    /// 不读取环境定位；存在但无效的配置传播错误，不能退回普通镜像配置。
    pub(super) fn read_active(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Option<Self>, EngineError> {
        let bytes = match MacosProtectedDocument::read(
            ACTIVE_CONFIGURATION,
            64 * 1024,
            deadline,
            checkpoint,
        ) {
            Ok(bytes) => bytes,
            Err(EngineError::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => {
                checkpoint()?;
                if Instant::now() >= deadline {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let settings = Self::decode(&bytes)?;
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(Some(settings))
    }

    /// 参数：bytes为保护文件的有界原字节；返回：完整合法材料，保留非UTF-8定位。
    /// 仅用于内容解析，不代表普通调用方能为来源不可信的JSON建立信任。
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, EngineError> {
        if bytes.is_empty() || bytes.len() > 64 * 1024 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let settings: Self =
            serde_json::from_slice(bytes).map_err(|_| BusinessError::InvalidArgument)?;
        if settings.schema_version != 1
            || settings.epoch_floor == 0
            || settings.active_epoch != settings.epoch_floor
        {
            return Err(BusinessError::InvalidArgument.into());
        }
        let trust = settings.trust()?;
        settings.expected()?;
        if !trust.contains_image_path(&settings.receipt_path) {
            return Err(BusinessError::InvalidArgument.into());
        }
        Ok(settings)
    }

    /// 参数：无；返回：由保护材料构造的独立信任，不从receipt或相邻文件自动注册密钥。
    pub(super) fn trust(&self) -> Result<MacosInstallationTrust, EngineError> {
        MacosInstallationTrust::from_host(
            self.public_key,
            self.active_epoch,
            Path::new(OsStr::from_bytes(&self.installation_root)),
        )
    }

    /// 参数：无；返回：独立镜像摘要和准确尺寸，不扩大128MiB镜像准入上限。
    pub(super) fn expected(&self) -> Result<ScanWorkerHostConfig, EngineError> {
        ScanWorkerHostConfig::from_expected_image(self.expected_sha256, self.expected_bytes)
    }
}
