//! 新生产入口的明确 ELF 资格；独立于旧 C 原子出生机制。

use super::linux_atomic_launcher::LinuxAtomicLauncher;
use std::ffi::CString;
use std::fs::File;
use std::path::{Path, PathBuf};

/// 绑定 root 实际编译的两个 ELF 与隔离目录，不以相邻材料建立安装信任。
/// 来源：原生 Rust tests-only artifact 与 File 生命周期。
pub(super) struct AtomicLauncherFixture {
    binary: PathBuf,
    replacement: PathBuf,
    directory: tempfile::TempDir,
}

impl AtomicLauncherFixture {
    /// 参数：无；返回：显式真实 artifact，不查 PATH、不自动编译、不跳过。
    pub(super) fn new() -> Self {
        Self {
            binary: artifact("DG_LINUX_ATOMIC_LAUNCH_FIXTURE"),
            replacement: artifact("DG_LINUX_ATOMIC_LAUNCH_REPLACEMENT"),
            directory: tempfile::tempdir().unwrap(),
        }
    }

    /// 参数：mode 为固定本机场景；返回：真实 held ELF 启动材料。
    pub(super) fn launcher(&self, mode: &str) -> LinuxAtomicLauncher {
        Self::from_file(File::open(&self.binary).unwrap(), mode, None)
    }

    /// 参数：image、mode、canary 为本案实际句柄与固定 argv；返回：显式环境材料。
    pub(super) fn from_file(image: File, mode: &str, canary: Option<i32>) -> LinuxAtomicLauncher {
        let mut args = vec![
            CString::new("qualified-test-image").unwrap(),
            CString::new(mode).unwrap(),
        ];
        if let Some(canary) = canary {
            args.push(CString::new(canary.to_string()).unwrap());
        }
        LinuxAtomicLauncher::prepare(
            image,
            args,
            vec![CString::new("DG_FIXED_ENV=sentinel").unwrap()],
        )
        .unwrap()
    }

    /// 参数：无；返回：替换路径的第二份真实 ELF。
    pub(super) fn replacement(&self) -> &Path {
        &self.replacement
    }

    /// 参数：无；返回：源镜像真实路径。
    pub(super) fn binary(&self) -> &Path {
        &self.binary
    }

    /// 参数：无；返回：本案专用隔离目录。
    pub(super) fn directory(&self) -> &Path {
        self.directory.path()
    }
}

fn artifact(name: &str) -> PathBuf {
    let path = match std::env::var_os(name) {
        Some(path) => PathBuf::from(path),
        None => panic!("root must compile and provide {name} explicitly"),
    };
    assert!(path.is_absolute() && path.is_file(), "qualified {name}");
    path
}
