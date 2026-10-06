use serde::{Deserialize, Serialize};

/// 经独立宿主信任签名验证的安装声明；声明不等同于文件历史或执行资格。
/// 来源：原生 Rust PF-06 macOS 安装材料合同；无 Java 对等对象。
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MacosInstallationClaims {
    pub(super) schema_version: u32,
    pub(super) installation_id: [u8; 16],
    pub(super) epoch: u64,
    pub(super) policy_version: u32,
    pub(super) target: String,
    pub(super) protocol_version: u32,
    pub(super) pinned_scanner_revision: String,
    pub(super) image_sha256: [u8; 32],
    pub(super) image_bytes: u64,
    pub(super) native_path: Vec<u8>,
    pub(super) volume_uuid: [u8; 16],
    pub(super) fsid: [i32; 2],
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) birth_seconds: i64,
    pub(super) birth_nanoseconds: u32,
    pub(super) fresh_from_birth: bool,
}

impl MacosInstallationClaims {
    /// 编码所有声明字段的固定顺序签名消息，字符串和路径采用 u32 小端长度前缀。
    /// 参数：self 为受界限约束的安装声明；返回：域分离后的签名消息，不授予执行资格。
    /// fresh_from_birth 只承载可信发行方声明，不从文件时间推断历史写句柄状态。
    pub(super) fn signing_message(&self) -> Vec<u8> {
        let mut message = b"diskgraph.macos.installation.receipt\0v1\0".to_vec();
        message.extend_from_slice(&self.schema_version.to_le_bytes());
        message.extend_from_slice(&self.installation_id);
        message.extend_from_slice(&self.epoch.to_le_bytes());
        message.extend_from_slice(&self.policy_version.to_le_bytes());
        append_bytes(&mut message, self.target.as_bytes());
        message.extend_from_slice(&self.protocol_version.to_le_bytes());
        append_bytes(&mut message, self.pinned_scanner_revision.as_bytes());
        message.extend_from_slice(&self.image_sha256);
        message.extend_from_slice(&self.image_bytes.to_le_bytes());
        append_bytes(&mut message, &self.native_path);
        message.extend_from_slice(&self.volume_uuid);
        for component in self.fsid {
            message.extend_from_slice(&component.to_le_bytes());
        }
        message.extend_from_slice(&self.device.to_le_bytes());
        message.extend_from_slice(&self.inode.to_le_bytes());
        message.extend_from_slice(&self.birth_seconds.to_le_bytes());
        message.extend_from_slice(&self.birth_nanoseconds.to_le_bytes());
        message.push(u8::from(self.fresh_from_birth));
        message
    }
}

fn append_bytes(message: &mut Vec<u8>, bytes: &[u8]) {
    // 所有调用材料来自最多 16 KiB 的 receipt 或受限可信发行路径，长度可用 u32 表示。
    message.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    message.extend_from_slice(bytes);
}
