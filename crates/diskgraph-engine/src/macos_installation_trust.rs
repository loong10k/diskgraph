use crate::EngineError;
use diskgraph_core::BusinessError;
use ed25519_dalek::VerifyingKey;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// 独立可信宿主预置的发行公钥、活跃安装 epoch 和原生根目录，不能从 receipt 自授信任。
/// 来源：原生 Rust PF-06 macOS 安装准入合同；无 Java 对等对象。
pub struct MacosInstallationTrust {
    pub(super) public_key: VerifyingKey,
    pub(super) active_epoch: u64,
    pub(super) installation_root: Vec<u8>,
}

impl MacosInstallationTrust {
    /// 从宿主独立可信配置构造安装信任，不读取相邻清单、不采用 TOFU。
    /// 参数：public_key 为 Ed25519 公钥，active_epoch 非零，installation_root 为受限绝对原生路径。
    /// 返回：已检查弱密钥和路径语法的信任材料；不验证实际 namespace 或授予执行资格。
    pub fn from_host(
        public_key: [u8; 32],
        active_epoch: u64,
        installation_root: &Path,
    ) -> Result<Self, EngineError> {
        let public_key =
            VerifyingKey::from_bytes(&public_key).map_err(|_| BusinessError::InvalidArgument)?;
        // dalek 解码检查与低阶点拒绝不宣称额外 canonical 编码保证。
        if public_key.is_weak() || active_epoch == 0 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let installation_root = native_bytes(installation_root)?;
        if !Self::is_valid_path(&installation_root) {
            return Err(BusinessError::InvalidArgument.into());
        }
        Ok(Self {
            public_key,
            active_epoch,
            installation_root,
        })
    }

    /// 检查原生 Unix 路径的无歧义词法形式，不进行文件系统解析。
    /// 参数：path 为无损路径字节；返回：是否绝对、至多 1024 字节且无 NUL、点段或空段。
    pub(super) fn is_valid_path(path: &[u8]) -> bool {
        if path.len() < 2 || path.len() > 1024 || path[0] != b'/' || path.contains(&0) {
            return false;
        }
        path[1..]
            .split(|byte| *byte == b'/')
            .all(|component| !component.is_empty() && component != b"." && component != b"..")
    }

    /// 检查安装镜像路径严格位于可信根目录内，并保持原生字节语义。
    /// 参数：path 为候选原生绝对路径；返回：合法且位于 root 后的真实分隔符之下时为 true。
    pub(super) fn contains_image_path(&self, path: &[u8]) -> bool {
        Self::is_valid_path(path)
            && path.starts_with(&self.installation_root)
            && path.get(self.installation_root.len()) == Some(&b'/')
    }
}

#[cfg(unix)]
fn native_bytes(path: &Path) -> Result<Vec<u8>, EngineError> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() > 1024 {
        return Err(BusinessError::InvalidArgument.into());
    }
    Ok(bytes.to_vec())
}

#[cfg(not(unix))]
fn native_bytes(_path: &Path) -> Result<Vec<u8>, EngineError> {
    // 非 Unix 平台不能将自己的路径编码冒充 macOS 原生路径。
    Err(BusinessError::Unsupported.into())
}
