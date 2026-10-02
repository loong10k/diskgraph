//! 在 macOS 观察 dataless 标志，其他平台诊断返回未知的保守探针。

use super::PlaceholderProbe;
use std::path::Path;

/// 在 macOS 观察 dataless 标志，其他平台诊断返回未知的保守探针。
/// 来源：原生 Rust diskgraph-engine::content::ConservativeProbe。
/// The production probe: honest about knowing nothing yet (tracked in the
/// platform ledger, `diskgraph_testkit::real_os_requirements`).
pub struct ConservativeProbe;

impl PlaceholderProbe for ConservativeProbe {
    fn is_placeholder(&self, path: &Path) -> bool {
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt;
            std::fs::symlink_metadata(path).is_ok_and(|meta| meta.st_flags() & 0x40000000 != 0)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            false
        }
    }
}
