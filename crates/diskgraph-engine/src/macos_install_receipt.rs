use crate::macos_installation_claims::MacosInstallationClaims;
use crate::macos_installation_trust::MacosInstallationTrust;
use crate::scan_worker_host_config::MAX_IMAGE_BYTES;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use ed25519_dalek::Signature;
use serde::Deserialize;

/// 有界安装 receipt 的私有 wire 对象，公钥和活跃 epoch 只由独立宿主信任提供。
/// 来源：原生 Rust PF-06 macOS 安装签名准入合同；无 Java 对等对象。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MacosInstallReceipt {
    claims: MacosInstallationClaims,
    signature: Vec<u8>,
}

impl MacosInstallReceipt {
    /// 验证有界 JSON、全部字段的严格 Ed25519 签名及宿主独立镜像绑定。
    /// 参数：bytes 至多 16 KiB，trust 来自可信宿主，expected 为独立完整摘要与准确长度。
    /// 返回：已认证声明；不验证文件历史、目录句柄、真实镜像或授予 exec 权限。
    pub(super) fn verify(
        bytes: &[u8],
        trust: &MacosInstallationTrust,
        expected: &ScanWorkerHostConfig,
    ) -> Result<MacosInstallationClaims, EngineError> {
        if bytes.is_empty() || bytes.len() > 16 * 1024 {
            return Err(BusinessError::InvalidArgument.into());
        }
        // struct Deserialize 拒绝重复及未知字段；from_slice 同时拒绝尾随第二对象/垃圾。
        let receipt: Self =
            serde_json::from_slice(bytes).map_err(|_| BusinessError::InvalidArgument)?;
        let signature = Signature::from_slice(&receipt.signature)
            .map_err(|_| BusinessError::InvalidArgument)?;
        trust
            .public_key
            .verify_strict(&receipt.claims.signing_message(), &signature)
            .map_err(|_| BusinessError::InvalidArgument)?;
        let claims = receipt.claims;
        if claims.schema_version != 1
            || claims.installation_id == [0; 16]
            || claims.epoch != trust.active_epoch
            || claims.policy_version != 1
            || claims.target != env!("DISKGRAPH_ENGINE_TARGET")
            || claims.protocol_version != 2
            || claims.pinned_scanner_revision != "158f9cc2f0b332194a3ffc5acec47760c99146d8"
            || claims.image_bytes == 0
            || claims.image_bytes > MAX_IMAGE_BYTES
            || claims.image_bytes != expected.expected_bytes
            || claims.image_sha256 != expected.expected_sha256
            || !trust.contains_image_path(&claims.native_path)
            || claims.volume_uuid == [0; 16]
            || claims.birth_nanoseconds >= 1_000_000_000
            || !claims.fresh_from_birth
        {
            return Err(BusinessError::InvalidArgument.into());
        }
        // 只返回经认证且绑定当前宿主策略的声明；后续 native lease 必须独立验证当前文件。
        Ok(claims)
    }
}
