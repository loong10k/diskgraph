//! 可信原生根定位的共享转换。

use crate::EngineError;

/// 从原生根定位取出路径。
/// 参数：root 为快照根节点。
/// 返回：原生路径字符串；文档 URI 返回非法参数。
/// The filesystem path a root node was indexed from.
pub(super) fn native_path(root: &diskgraph_core::DiskNode) -> Result<String, EngineError> {
    match &root.locator {
        diskgraph_core::ResourceLocator::NativePath(path) => Ok(path.clone()),
        diskgraph_core::ResourceLocator::DocumentUri(_) => Err(EngineError::Business(
            diskgraph_core::BusinessError::InvalidArgument,
        )),
    }
}
