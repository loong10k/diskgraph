//! 当前进程新捕获扫描节点的明确编码来源，禁止用于历史字节猜测。

use crate::EngineError;
use diskgraph_core::{BusinessError, LocatorEncoding, LocatorKind, QualifiedLocator};
use diskgraph_disktree::NodeV2;

/// 将本次原生扫描捕获的定位附上已知宿主编码。
/// 参数：node 必须来自当前进程的 convert_tree，而非旧数据库或请求输入。
/// 返回：明确编码的原始定位；异常字段明确拒绝，不从展示路径恢复字节。
pub(super) fn qualify_scan_locator(node: &NodeV2) -> Result<QualifiedLocator, EngineError> {
    if node.locator.kind != LocatorKind::NativePath {
        return Err(BusinessError::Unsupported.into());
    }
    #[cfg(unix)]
    let encoding = LocatorEncoding::UnixBytes;
    #[cfg(windows)]
    let encoding = LocatorEncoding::WindowsUtf16Le;
    #[cfg(not(any(unix, windows)))]
    return Err(BusinessError::Unsupported.into());
    #[cfg(any(unix, windows))]
    QualifiedLocator::from_parts(
        node.locator.kind,
        encoding,
        node.locator
            .raw_bytes()
            .map_err(|_| BusinessError::Unsupported)?,
        node.locator.display.clone(),
    )
    .map_err(|_| BusinessError::Unsupported.into())
}
