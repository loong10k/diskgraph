//! raw_path：既有文件操作职责的原生 Rust 实现。
use std::path::PathBuf;

/// 从原生定位器提取已有路径字节的内部兼容接口。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::RawPath`，保留既有语义。
/// A read-only locator helper used by the ops layer and the store.
pub(super) trait RawPath {
    /// 读取原有定位键。
    /// 参数：self 为定位器。
    /// 返回：NativePath 对应的 PathBuf；DocumentUri 为 None。
    fn raw_path(&self) -> Option<PathBuf>;
}

impl RawPath for diskgraph_core::ResourceLocator {
    /// 取得定位器已有路径键。
    /// 参数：self 为资源定位器。
    /// 返回：NativePath 的路径副本；DocumentUri 为 None。
    fn raw_path(&self) -> Option<PathBuf> {
        match self {
            diskgraph_core::ResourceLocator::NativePath(path) => Some(PathBuf::from(path)),
            diskgraph_core::ResourceLocator::DocumentUri(_) => None,
        }
    }
}
