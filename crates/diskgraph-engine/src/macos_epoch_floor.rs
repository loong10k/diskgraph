use crate::EngineError;
use crate::macos_host_settings::MacosHostSettings;
use crate::macos_installation_lock::MacosInstallationLock;
use crate::macos_protected_document::MacosProtectedDocument;
use diskgraph_core::BusinessError;
use serde::Deserialize;
use std::time::Instant;

const FLOOR: &[u8] = b"/Library/Application Support/DiskGraph/scan-worker/epoch-floor.json";

/// 独立持久版本下限及活跃配置绑定；不从receipt或普通数据库重建。
/// 来源：原生Rust PF-06安装更新合同，无Java对等对象。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MacosEpochFloor {
    schema_version: u32,
    epoch: u64,
    active_sha256: [u8; 32],
}
impl MacosEpochFloor {
    /// 参数：无；返回：已验证下限记录中的 epoch。
    /// 返回：已严格解码的独立epoch；仅供可信恢复者定位该代固定pending材料。
    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }
    /// 参数：deadline/checkpoint 为原期限；返回：确认下限尚不存在，或残留下限、预算及读取错误。
    ///
    /// 参数：原期限/检查点；仅在active缺失时确认下限也不存在，残留下限必须拒绝。
    pub(super) fn ensure_unconfigured(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        match MacosProtectedDocument::read(FLOOR, 4096, deadline, checkpoint) {
            Err(EngineError::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => {
                checkpoint()?;
                if Instant::now() >= deadline {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                Ok(())
            }
            Ok(_) => Err(BusinessError::Conflict.into()),
            Err(error) => Err(error),
        }
    }

    /// 参数：原共享锁、期限与取消检查点；返回：保护文件内的独立下限。
    pub(super) fn read(
        _guard: &MacosInstallationLock,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        let bytes = MacosProtectedDocument::read(FLOOR, 4096, deadline, checkpoint)?;
        let floor = Self::decode(&bytes)?;
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(floor)
    }
    /// 参数：有界原始JSON；返回：严格记录，未知、重复字段及非法版本均拒绝。
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, EngineError> {
        if bytes.is_empty() || bytes.len() > 4096 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let floor: Self =
            serde_json::from_slice(bytes).map_err(|_| BusinessError::InvalidArgument)?;
        if floor.schema_version != 1 || floor.epoch == 0 || floor.active_sha256 == [0; 32] {
            return Err(BusinessError::InvalidArgument.into());
        }
        Ok(floor)
    }
    /// 参数：同锁内读取的活跃配置；返回：一致性验证，损坏或回滚返回：冲突。
    pub(super) fn verify(&self, settings: &MacosHostSettings) -> Result<(), EngineError> {
        if settings.active_epoch != self.epoch || settings.binding_digest()? != self.active_sha256 {
            return Err(BusinessError::Conflict.into());
        }
        Ok(())
    }
}
