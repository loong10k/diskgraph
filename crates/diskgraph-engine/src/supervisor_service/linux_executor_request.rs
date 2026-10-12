use super::linux_executor_wire::LinuxExecutorWire;
use crate::native_deadline::ClockStamp;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 用户executor的有限出生请求；来源：PF-06 guardian/executor合同，无Java对等对象。
/// 不含容量目录/namespace/slot能力；解析不认证发送者，不采用FD或建立Engine。
pub struct LinuxExecutorRequest {
    uid: u32,
    gid: u32,
    group_count: u32,
    groups_sha256: [u8; 32],
    worker_sha256: [u8; 32],
    worker_bytes: u64,
    data_dir: PathBuf,
    deadline: ClockStamp,
}

impl LinuxExecutorRequest {
    /// 参数：bytes为最多4096字节的原请求；返回：字段/原期限验证通过的请求或拒绝。
    /// 必须另通过原父逐消息内核凭据与JSON摘要认证，不兼容降级为旧service材料。
    pub fn decode(bytes: &[u8]) -> Result<Self, EngineError> {
        if bytes.is_empty() || bytes.len() > 4096 {
            return Err(BusinessError::PermissionDenied.into());
        }
        // 严格字段反序列化拒绝重复UID、旧容量材料及客户选取的授权模式。
        let wire: LinuxExecutorWire =
            serde_json::from_slice(bytes).map_err(|_| BusinessError::PermissionDenied)?;
        let valid_identity = |id| id != 0 && id != u32::MAX;
        if wire.schema_version != 1
            || !valid_identity(wire.uid)
            || !valid_identity(wire.gid)
            || wire.worker_fd != 7
            || wire.control_fd != 6
            || wire.group_count > 65536
            || wire.worker_bytes == 0
            || wire.worker_bytes > 64 << 20
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        let digest = |text: &str| -> Result<[u8; 32], EngineError> {
            if text.len() != 64
                || !text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(BusinessError::PermissionDenied.into());
            }
            hex::decode(text)
                .map_err(|_| BusinessError::PermissionDenied)?
                .try_into()
                .map_err(|_| BusinessError::PermissionDenied.into())
        };
        let text = &wire.data_dir;
        if text.len() > 1024
            || !text.starts_with('/')
            || text.ends_with('/')
            || text.contains('\0')
            || text
                .split('/')
                .skip(1)
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        let groups_sha256 = digest(&wire.groups_sha256)?;
        let worker_sha256 = digest(&wire.worker_sha256)?;
        let deadline = wire.deadline;
        // 使用同一原生截止值扣除排队时间；解析不重新给予完整启动预算。
        deadline.adopt(Duration::from_secs(30))?;
        Ok(Self {
            uid: wire.uid,
            gid: wire.gid,
            group_count: wire.group_count,
            groups_sha256,
            worker_sha256,
            worker_bytes: wire.worker_bytes,
            data_dir: PathBuf::from(wire.data_dir),
            deadline,
        })
    }

    /// 参数：无；返回：请求绑定的真实UID/GID，仍须与原进程完整凭据核对。
    pub fn identity(&self) -> (u32, u32) {
        (self.uid, self.gid)
    }

    /// 参数：无；返回：完整辅助组数量及摘要，不能以账户数据库或省略组替代核对。
    pub fn group_binding(&self) -> (u32, [u8; 32]) {
        (self.group_count, self.groups_sha256)
    }

    /// 参数：无；返回：原请求的数据目录借用，不提供客户文件权限或容量能力。
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 参数：无；返回：原已验证的worker镜像预期，不通过客户路径重新打开镜像。
    pub fn worker_expectation(&self) -> Result<ScanWorkerHostConfig, EngineError> {
        ScanWorkerHostConfig::from_expected_image(self.worker_sha256, self.worker_bytes)
    }

    /// 参数：无；返回：原绝对期限剩余值或已过期错误，调用不会刷新预算。
    pub fn deadline(&self) -> Result<Instant, EngineError> {
        self.deadline.adopt(Duration::from_secs(30))
    }
}
