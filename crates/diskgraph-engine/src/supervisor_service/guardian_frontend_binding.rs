use serde::Deserialize;
use std::io;

/// 受信root安装的公开客户/guardian身份绑定；来源：PF-06原连接guardian合同，无Java对等对象。
/// 不携带私有容量或出生能力。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GuardianFrontendBinding {
    schema_version: u32,
    socket_path: String,
    frontend_uid: u32,
    guardian_uid: u32,
    guardian_gid: u32,
}

impl GuardianFrontendBinding {
    /// 参数：有界原文件正文与实际socket路径；返回：严格安装绑定或拒绝，不降级到旧broker。
    pub(super) fn decode(bytes: &[u8], socket: &str) -> io::Result<Self> {
        if bytes.is_empty() || bytes.len() > 4096 {
            return Err(io::Error::other("invalid guardian frontend binding size"));
        }
        let binding: Self = serde_json::from_slice(bytes)?;
        let valid = |id| id != 0 && id != u32::MAX;
        if binding.schema_version != 1
            || binding.socket_path != socket
            || !valid(binding.frontend_uid)
            || !valid(binding.guardian_uid)
            || !valid(binding.guardian_gid)
            || binding.frontend_uid == binding.guardian_uid
        {
            return Err(io::Error::other("invalid guardian frontend binding"));
        }
        Ok(binding)
    }

    /// 参数：无；返回：安装者指定客户UID，不从公开消息推断。
    pub(super) fn frontend_uid(&self) -> u32 {
        self.frontend_uid
    }

    /// 参数：无；返回：安装者指定的永久guardian身份。
    pub(super) fn guardian_identity(&self) -> (u32, u32) {
        (self.guardian_uid, self.guardian_gid)
    }
}
