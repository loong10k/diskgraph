use crate::EngineError;
use crate::native_deadline::ClockStamp;
use diskgraph_core::BusinessError;
use serde::Deserialize;
use std::path::PathBuf;

/// root 出生层签发的单次 doctor 启动材料；来源：原生 PF-06，无 Java 对等对象。
/// JSON 摘要必须由私有通道的 root 内核凭据确认；解析成功本身不授予身份。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinuxSupervisorBirthRequest {
    pub(crate) schema_version: u16,
    pub(crate) nonce: String,
    pub(crate) service_uid: u32,
    pub(crate) frontend_uid: u32,
    pub(crate) state_root: PathBuf,
    pub(crate) data_dir: PathBuf,
    pub(crate) worker_path: PathBuf,
    pub(crate) worker_sha256: String,
    pub(crate) worker_bytes: u64,
    pub(crate) deadline: ClockStamp,
    #[serde(default)]
    pub(crate) control_fd: Option<u32>,
    #[serde(default)]
    pub(crate) service_gid: Option<u32>,
}

impl LinuxSupervisorBirthRequest {
    /// 参数：bytes 为原出生层 JSON 字节；返回：启动材料或明确拒绝，不读取路径或创建数据库。
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, EngineError> {
        if bytes.is_empty() || bytes.len() > 4096 {
            return Err(BusinessError::PermissionDenied.into());
        }
        let request: Self =
            serde_json::from_slice(bytes).map_err(|_| BusinessError::PermissionDenied)?;
        let valid_id = |uid| uid != 0 && uid != u32::MAX;
        let valid_digest = |text: &str| {
            text.len() == 64
                && text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        // 固定安装位置不接受路径别名或任意额外命令；解析阶段尚不打开任何候选对象。
        let valid_path = |path: &PathBuf| {
            path.to_str().is_some_and(|s| {
                s.len() <= 1024
                    && s.starts_with('/')
                    && !s.ends_with('/')
                    && s.split('/')
                        .skip(1)
                        .all(|c| !c.is_empty() && c != "." && c != "..")
            })
        };
        let valid_control = match (
            request.schema_version,
            request.control_fd,
            request.service_gid,
        ) {
            (1, None, None) => true,
            (2, Some(fd), Some(gid)) => (3..=i32::MAX as u32).contains(&fd) && valid_id(gid),
            (3, Some(6), Some(gid)) => valid_id(gid),
            _ => false,
        };
        if !valid_control
            || !valid_id(request.service_uid)
            || !valid_id(request.frontend_uid)
            || request.service_uid == request.frontend_uid
            || !valid_digest(&request.nonce)
            || !valid_digest(&request.worker_sha256)
            || request.worker_bytes == 0
            || request.worker_bytes > 64 << 20
            || !valid_path(&request.state_root)
            || !valid_path(&request.data_dir)
            || !valid_path(&request.worker_path)
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        Ok(request)
    }
}

#[cfg(test)]
#[path = "linux_supervisor_birth_request_tests.rs"]
mod tests;
