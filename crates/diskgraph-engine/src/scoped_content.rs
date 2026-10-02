use std::fs::{File, Metadata};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

use diskgraph_core::BusinessError;

use crate::EngineError;
use crate::content::PlaceholderProbe;

/// 跨平台内容准备与检查租约；来源：DiskGraph CT-01/02/05 原生内容契约。
/// Unix 保留已有检查；Windows 仅通过逐组件属性句柄获取后申请数据读取。
pub(crate) struct ScopedContent {
    pub(crate) metadata: Metadata,
    pub(crate) file: Option<File>,
    #[cfg(unix)]
    identity: Option<String>,
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    lease: crate::windows_scoped_file::WindowsScopedFile,
}

impl ScopedContent {
    /// 进入本线程原生策略；返回 RAII guard，无法提供策略时拒绝内容访问。
    /// 参数：无；作用于调用线程。
    /// 返回：平台原生线程策略 guard。macOS 禁止 dataless 物化，Windows 暴露占位属性；provider 不下载须另行验收，无法提供策略返回 unsupported 或 I/O 错误。
    pub(crate) fn hydration_guard() -> Result<diskgraph_disktree::HydrationGuard, EngineError> {
        diskgraph_disktree::HydrationGuard::enter().map_err(Self::hydration_error)
    }

    fn hydration_error(error: std::io::Error) -> EngineError {
        if error.kind() == std::io::ErrorKind::Unsupported {
            EngineError::Business(BusinessError::Unsupported)
        } else {
            error.into()
        }
    }

    /// 对根下路径执行原生获取；返回占位结果时 file 为 None，绝不读取数据。
    /// 参数：root/path 为注册根与精确路径，probe 为占位诊断。
    /// 返回：原生租约与元数据，占位对象无数据句柄；获取或验证失败返回错误。
    pub(crate) fn open(
        root: &Path,
        path: &Path,
        probe: &dyn PlaceholderProbe,
    ) -> Result<Self, EngineError> {
        #[cfg(unix)]
        {
            crate::content::ensure_inside_scope(root, path)?;
            let metadata = std::fs::symlink_metadata(path)?;
            crate::content::ensure_plain_file(path, &metadata)?;
            let identity = crate::content::file_identity(&metadata);
            let file = if probe.is_placeholder(path) {
                None
            } else {
                let file = crate::scoped_file::open_scoped(root, path)?;
                if crate::content::file_identity(&file.metadata()?) != identity {
                    return Err(EngineError::Business(BusinessError::Conflict));
                }
                Some(file)
            };
            Ok(Self {
                metadata,
                file,
                identity,
                path: path.to_path_buf(),
            })
        }
        #[cfg(windows)]
        {
            let lease = crate::windows_scoped_file::WindowsScopedFile::open(root, path)?;
            let metadata = lease.metadata()?;
            let file = if lease.state.placeholder() || probe.is_placeholder(path) {
                None
            } else {
                Some(lease.open_data()?)
            };
            Ok(Self {
                metadata,
                file,
                lease,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (root, path, probe);
            Err(EngineError::Business(BusinessError::Unsupported))
        }
    }

    /// 核验已打开文件与原生租约；返回 false 时部分内容及摘要不能确认稳定。
    /// 参数：file 为本轮已打开的数据句柄。
    /// 返回：身份和版本仍符合原租约时 true；失败结果不能确认内容稳定。
    pub(crate) fn matches(&self, file: &File) -> bool {
        #[cfg(unix)]
        {
            file.metadata()
                .is_ok_and(|metadata| crate::content::file_identity(&metadata) == self.identity)
                && crate::content::identity_stable(&self.identity, &self.path)
        }
        #[cfg(windows)]
        {
            self.lease.matches(file)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = file;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScopedContent;
    use crate::EngineError;
    use diskgraph_core::BusinessError;

    #[test]
    fn a_missing_native_guard_api_reports_publicly_unsupported() {
        let error = ScopedContent::hydration_error(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "missing native mode export",
        ));
        assert!(matches!(
            error,
            EngineError::Business(BusinessError::Unsupported)
        ));
    }

    #[test]
    fn other_native_guard_failures_preserve_io_error_semantics() {
        let error = ScopedContent::hydration_error(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "native policy rejected",
        ));
        assert!(
            matches!(error, EngineError::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
    }
}
