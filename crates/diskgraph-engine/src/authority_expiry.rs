//! 固定请求能力到期时间的共享返回门禁。
use crate::EngineError;
use diskgraph_core::BusinessError;

/// 核验同次请求固定的能力到期时间。
/// 参数：expiry 为 Unix 秒时间戳；None 保留可信本机无 token 的兼容模式。
/// 返回：尚未到期则成功，到期或系统时钟不可解释则拒权；不延长锁等待或执行期限。
pub(super) fn check_authority_expiry(expiry: Option<u64>) -> Result<(), EngineError> {
    if let Some(expiry) = expiry {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?
            .as_secs();
        if now >= expiry {
            return Err(BusinessError::PermissionDenied.into());
        }
    }
    Ok(())
}
