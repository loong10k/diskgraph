use crate::macos_epoch_floor::MacosEpochFloor;
use crate::macos_filesystem_state::MacosFilesystemState;
use crate::macos_host_settings::MacosHostSettings;
use crate::macos_installation_claims::MacosInstallationClaims;
use crate::macos_installation_files::MacosInstallationFiles as Files;
use crate::macos_installation_lease::{MacosInstallationLease, open_namespace};
use crate::macos_installation_lock::MacosInstallationLock;
use crate::macos_protected_document::MacosProtectedDocument;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::File;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::time::Instant;

const BASE: &str = "/Library/Application Support/DiskGraph/scan-worker";
const LOCK: &[u8] = b"/Library/Application Support/DiskGraph/scan-worker/installation.lock";
const FLOOR: &[u8] = b"/Library/Application Support/DiskGraph/scan-worker/epoch-floor.json";
const ACTIVE: &[u8] = b"/Library/Application Support/DiskGraph/scan-worker/active.json";

/// 仅可信root发行者可调用的fresh安装与单调更新事务，不执行任何发布镜像。
/// 来源：原生Rust PF-06独立持久下限及Apple目录同步合同；无Java对等对象。
/// 前置条件：可信bootstrap已逐组件建立固定保护目录和永久空installation.lock。
/// 不创建信任根、不chown旧文件；中断状态失败关闭，恢复不得降低已发布floor。
pub struct MacosInstallationPublisher;

impl MacosInstallationPublisher {
    /// 建立或核验可信安装程序的固定保护布局，不发布镜像或授予请求权限。
    /// 参数：deadline/checkpoint 为原安装期限及取消；返回：持久布局核验成功或原失败。
    /// 真实和有效 UID 均须为 root；不提供自动提权，不改写已有对象的权限或内容。
    pub fn prepare(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        crate::macos_installation_bootstrap::MacosInstallationBootstrap::prepare(
            deadline, checkpoint,
        )
    }

    /// 仅root完成独立floor已经承诺的一代；不接受新私钥、摘要、epoch或任意恢复路径。
    /// 参数：原期限/取消检查点；返回：该代active已持久化或原失败，永不改写floor。
    /// pending已消费时只接受当前active完全匹配floor；损坏/缺失下限拒绝自动重建。
    pub fn recover(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        Files::check(deadline, checkpoint)?;
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return Err(BusinessError::Unsupported.into());
        }
        let guard = MacosInstallationLock::acquire_exclusive(deadline, checkpoint)?;
        let floor_bytes = MacosProtectedDocument::read(FLOOR, 4096, deadline, checkpoint)?;
        let floor = MacosEpochFloor::decode(&floor_bytes)?;
        let name = format!("epoch-{:020}", floor.epoch());
        let pending_path = format!("{BASE}/versions/{name}/active.pending.json");
        let pending = read_optional(pending_path.as_bytes(), 64 * 1024, deadline, checkpoint)?;
        let candidate = match &pending {
            Some(bytes) => bytes.clone(),
            None => MacosProtectedDocument::read(ACTIVE, 64 * 1024, deadline, checkpoint)?,
        };
        let settings = Self::validate_recovery(&floor_bytes, &candidate)?;
        let receipt =
            MacosProtectedDocument::read(&settings.receipt_path, 16 * 1024, deadline, checkpoint)?;
        let lease = MacosInstallationLease::admit(
            &receipt,
            &settings.trust()?,
            &settings.expected()?,
            deadline,
            checkpoint,
        )?;
        lease.validate_loader(deadline, checkpoint)?;
        // 新打开句柄只作同卷flush锚点，不替代lease原FD的内容核验。
        let (_, image, _) = open_namespace(lease.native_path().as_bytes(), deadline, checkpoint)?;
        let (mut ancestors, _lock_identity, _) = open_namespace(LOCK, deadline, checkpoint)?;
        let (base, _) = ancestors.pop().ok_or(BusinessError::Unsupported)?;
        let versions = Files::open_directory(&base, c"versions")?;
        let name = CString::new(name).map_err(|_| BusinessError::InvalidArgument)?;
        let version = Files::open_directory(&versions, &name)?;
        MacosFilesystemState::capture(&versions, true)?;
        MacosFilesystemState::capture(&version, true)?;
        Files::durable(&image, &[&version, &versions, &base], deadline, checkpoint)?;
        // 中断可能发生在floor rename之后、flush之前；先同步原floor再完成active。
        let (_, floor_anchor, _) = open_namespace(FLOOR, deadline, checkpoint)?;
        Files::durable(
            &floor_anchor,
            &[&version, &versions, &base],
            deadline,
            checkpoint,
        )?;
        let current_floor = MacosProtectedDocument::read(FLOOR, 4096, deadline, checkpoint)?;
        if current_floor != floor_bytes {
            return Err(BusinessError::Conflict.into());
        }
        if pending.is_some() {
            Files::complete_active(&base, &version, &image, deadline, checkpoint)?;
        } else {
            // active已rename的中断也须flush原active数据与固定目录项。
            let (_, active_anchor, _) = open_namespace(ACTIVE, deadline, checkpoint)?;
            Files::durable(&active_anchor, &[&base], deadline, checkpoint)?;
        }
        if MacosHostSettings::read_authorized(&guard, deadline, checkpoint)? != settings {
            return Err(BusinessError::Conflict.into());
        }
        Files::check(deadline, checkpoint)
    }

    /// 参数：独立下限与该代候选配置；返回：只可用于固定布局恢复的已认证配置。
    /// 摘要覆盖全部信任字段，不允许恢复者改写预期或用任意旧版本receipt补齐。
    pub(super) fn validate_recovery(
        floor: &[u8],
        candidate: &[u8],
    ) -> Result<MacosHostSettings, EngineError> {
        let floor = MacosEpochFloor::decode(floor)?;
        let settings = MacosHostSettings::decode(candidate)?;
        floor.verify(&settings)?;
        let expected_root = format!("{BASE}/versions");
        let expected_receipt = format!("{expected_root}/epoch-{:020}/receipt.json", floor.epoch());
        if settings.installation_root != expected_root.as_bytes()
            || settings.receipt_path != expected_receipt.as_bytes()
        {
            return Err(BusinessError::Conflict.into());
        }
        Ok(settings)
    }

    /// 参数：held源、独立预期、可信发行私钥、递增epoch、非零安装ID及原期限。
    /// 返回：完整发布或原失败；失败可能已持久发布新floor，调用方不得回滚或复用旧epoch。
    /// 私钥/预期不能取自请求或相邻文件；本接口只供固定可信安装程序，不供普通CLI/MCP。
    pub fn publish(
        source: &File,
        expected: &ScanWorkerHostConfig,
        signing_key: &SigningKey,
        epoch: u64,
        installation_id: [u8; 16],
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        Files::check(deadline, checkpoint)?;
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return Err(BusinessError::Unsupported.into());
        }
        if epoch == 0 || installation_id == [0; 16] {
            return Err(BusinessError::InvalidArgument.into());
        }
        let _guard = MacosInstallationLock::acquire_exclusive(deadline, checkpoint)?;
        let previous_floor = read_optional(FLOOR, 4096, deadline, checkpoint)?;
        let previous_active = read_optional(ACTIVE, 64 * 1024, deadline, checkpoint)?;
        Self::validate_previous(previous_floor.as_deref(), previous_active.as_deref(), epoch)?;

        // 复用同一严格根解析；最后一个祖先就是固定base，不能create_dir_all跟随别名。
        let (mut ancestors, _lock_identity, _) = open_namespace(LOCK, deadline, checkpoint)?;
        let (base, base_state) = ancestors.pop().ok_or(BusinessError::Unsupported)?;
        if base_state.mode & 0o005 != 0o005 {
            return Err(BusinessError::Unsupported.into());
        }
        let versions = match Files::open_directory(&base, c"versions") {
            Ok(directory) => directory,
            Err(EngineError::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => {
                Files::create_directory(&base, c"versions")?
            }
            Err(error) => return Err(error),
        };
        let versions_state = MacosFilesystemState::capture(&versions, true)?;
        if versions_state.mode & 0o005 != 0o005 {
            return Err(BusinessError::Unsupported.into());
        }
        Files::check(deadline, checkpoint)?;
        let version_name = format!("epoch-{epoch:020}");
        let component =
            CString::new(version_name.as_bytes()).map_err(|_| BusinessError::InvalidArgument)?;
        let version = Files::create_directory(&versions, &component)?;
        MacosFilesystemState::capture(&version, true)?;
        let image = copy_fresh_image(source, expected, &version, deadline, checkpoint)?;
        let state = MacosFilesystemState::capture(&image, false)?;
        Files::check(deadline, checkpoint)?;
        crate::macos_load_policy::MacosLoadPolicy::validate(
            &image,
            expected.expected_bytes,
            deadline,
            checkpoint,
        )?;
        let installation_root = format!("{BASE}/versions").into_bytes();
        let image_path =
            format!("{BASE}/versions/{version_name}/diskgraph-scan-worker").into_bytes();
        let receipt_path = format!("{BASE}/versions/{version_name}/receipt.json").into_bytes();
        let claims = MacosInstallationClaims {
            schema_version: 1,
            installation_id,
            epoch,
            policy_version: 1,
            target: env!("DISKGRAPH_ENGINE_TARGET").into(),
            protocol_version: 2,
            pinned_scanner_revision: "158f9cc2f0b332194a3ffc5acec47760c99146d8".into(),
            image_sha256: expected.expected_sha256,
            image_bytes: expected.expected_bytes,
            native_path: image_path,
            volume_uuid: state.volume_uuid,
            fsid: state.fsid,
            device: state.device,
            inode: state.inode,
            birth_seconds: state.birth_seconds,
            birth_nanoseconds: state.birth_nanoseconds,
            fresh_from_birth: true,
        };
        // copy_fresh_image已显式关闭唯一发行writer，此后才签发出生历史声明。
        let signature = signing_key
            .sign(&claims.signing_message())
            .to_bytes()
            .to_vec();
        let receipt =
            serde_json::to_vec(&serde_json::json!({"claims": claims, "signature": signature}))
                .map_err(|_| BusinessError::InvalidArgument)?;
        if receipt.len() > 16 * 1024 {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let _receipt = Files::write_fresh(
            &version,
            c"receipt.json",
            &receipt,
            0o444,
            deadline,
            checkpoint,
        )?;
        let active = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "public_key": signing_key.verifying_key().to_bytes(),
            "active_epoch": epoch,
            "epoch_floor": epoch,
            "expected_sha256": expected.expected_sha256,
            "expected_bytes": expected.expected_bytes,
            "installation_root": installation_root,
            "receipt_path": receipt_path,
        }))
        .map_err(|_| BusinessError::InvalidArgument)?;
        let settings = MacosHostSettings::decode(&active)?;
        // 用消费者同一验签/原FD/摘要/加载策略复验，签发本身不代表准入已成立。
        let lease = MacosInstallationLease::admit(
            &receipt,
            &settings.trust()?,
            expected,
            deadline,
            checkpoint,
        )?;
        lease.validate_loader(deadline, checkpoint)?;
        let floor = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "epoch": epoch, "active_sha256": settings.binding_digest()?,
        }))
        .map_err(|_| BusinessError::InvalidArgument)?;
        MacosEpochFloor::decode(&floor)?.verify(&settings)?;
        Files::write_fresh(
            &version,
            c"active.pending.json",
            &active,
            0o444,
            deadline,
            checkpoint,
        )?;
        Files::write_fresh(
            &version,
            c"epoch-floor.pending.json",
            &floor,
            0o444,
            deadline,
            checkpoint,
        )?;
        // 先保证新版本全部内容及目录链接持久，再触碰固定floor/active。
        Files::durable(&image, &[&version, &versions, &base], deadline, checkpoint)?;
        let (current_ancestors, _current_lock, _) = open_namespace(LOCK, deadline, checkpoint)?;
        let current = &current_ancestors
            .last()
            .ok_or(BusinessError::Unsupported)?
            .1;
        if !base_state.same_directory_binding(current) {
            return Err(BusinessError::Conflict.into());
        }
        Files::publish_pair(&base, &version, &image, deadline, checkpoint)?;
        let observed = MacosHostSettings::read_authorized(&_guard, deadline, checkpoint)?;
        if observed != settings {
            return Err(BusinessError::Conflict.into());
        }
        Files::check(deadline, checkpoint)
    }

    /// 参数：同锁内旧floor/active原材料和新epoch；返回：首装/严格递增许可。
    /// 单项缺失、损坏、中断不一致均失败关闭，不从receipt或目录列表重建下限。
    pub(super) fn validate_previous(
        floor: Option<&[u8]>,
        active: Option<&[u8]>,
        epoch: u64,
    ) -> Result<(), EngineError> {
        if epoch == 0 {
            return Err(BusinessError::InvalidArgument.into());
        }
        match (floor, active) {
            (None, None) => Ok(()),
            (Some(floor), Some(active)) => {
                let floor = MacosEpochFloor::decode(floor)?;
                let active = MacosHostSettings::decode(active)?;
                floor.verify(&active)?;
                if epoch <= active.active_epoch {
                    return Err(BusinessError::Conflict.into());
                }
                Ok(())
            }
            _ => Err(BusinessError::Conflict.into()),
        }
    }
    /// 参数：source 为原源句柄，expected 为摘要与长度，version 为目标目录，deadline/checkpoint 为原期限；返回：真实复制并验证的文件句柄或原错误，不提供生产签发旁路。
    ///
    /// 普通隔离夹具只验证真实复制原语；不提供生产root准入或签发旁路。
    #[cfg(test)]
    pub(super) fn copy_for_test(
        source: &File,
        expected: &ScanWorkerHostConfig,
        version: &File,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<File, EngineError> {
        copy_fresh_image(source, expected, version, deadline, checkpoint)
    }
}

fn read_optional(
    path: &[u8],
    cap: usize,
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<Option<Vec<u8>>, EngineError> {
    match MacosProtectedDocument::read(path, cap, deadline, checkpoint) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(EngineError::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => {
            Files::check(deadline, checkpoint)?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn copy_fresh_image(
    source: &File,
    expected: &ScanWorkerHostConfig,
    version: &File,
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<File, EngineError> {
    Files::check(deadline, checkpoint)?;
    let metadata = source.metadata()?;
    if !metadata.is_file() || metadata.len() != expected.expected_bytes {
        return Err(BusinessError::Conflict.into());
    }
    let mut writer = Files::fresh_writer(version, c"diskgraph-scan-worker")?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut offset = 0_u64;
    let mut hash = Sha256::new();
    while offset < expected.expected_bytes {
        Files::check(deadline, checkpoint)?;
        let count = (expected.expected_bytes - offset).min(buffer.len() as u64) as usize;
        let read = match source.read_at(&mut buffer[..count], offset) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        Files::check(deadline, checkpoint)?;
        if read == 0 {
            return Err(BusinessError::Conflict.into());
        }
        writer.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
        offset += read as u64;
    }
    Files::check(deadline, checkpoint)?;
    let mut extra = [0_u8; 1];
    let excess = source.read_at(&mut extra, offset)?;
    Files::check(deadline, checkpoint)?;
    let actual: [u8; 32] = hash.finalize().into();
    if excess != 0 || actual != expected.expected_sha256 {
        return Err(BusinessError::Conflict.into());
    }
    Files::finish_writer(writer, 0o555, deadline, checkpoint)?;
    Files::read_only(version, c"diskgraph-scan-worker")
}
