use crate::EngineError;
use diskgraph_core::BusinessError;

pub(crate) const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;

/// 宿主独立信任来源给出的镜像预期值，不从远程协议或相邻清单建立信任。
/// 来源：原生 Rust PF-06 受信宿主安装材料合同；无 Java 对等对象。
#[derive(Clone, Copy, Debug)]
pub struct ScanWorkerHostConfig {
    pub(crate) expected_sha256: [u8; 32],
    pub(crate) expected_bytes: u64,
}

impl ScanWorkerHostConfig {
    /// 参数：expected_sha256 和 expected_bytes 来自受信安装/宿主配置，不来自请求。
    /// 返回：不可变预期值；空镜像拒绝，无界镜像返回原预算错误。不授予执行权限。
    pub fn from_expected_image(
        expected_sha256: [u8; 32],
        expected_bytes: u64,
    ) -> Result<Self, EngineError> {
        if expected_bytes == 0 {
            return Err(BusinessError::InvalidArgument.into());
        }
        if expected_bytes > MAX_IMAGE_BYTES {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(Self {
            expected_sha256,
            expected_bytes,
        })
    }
}
