//! 独立 C artifact 的明确资格；来源：Linux 原子出生机制探针，不是安装信任证明。

use std::path::{Path, PathBuf};

/// 绑定 root 明确编译的两份 ELF 与本案隔离目录，不查 PATH 或隐式编译。
/// 来源：原生 Rust tests-only artifact 资格与 tempfile 生命周期。
pub(super) struct AtomicBirthFixture {
    binary: PathBuf,
    replacement: PathBuf,
    directory: tempfile::TempDir,
}

impl AtomicBirthFixture {
    /// 取得显式原生 artifact；参数：无；返回：缺失或非绝对文件时真实失败的夹具。
    pub(super) fn new() -> Self {
        Self {
            binary: artifact("DG_LINUX_ATOMIC_BIRTH_PROBE"),
            replacement: artifact("DG_LINUX_ATOMIC_BIRTH_REPLACEMENT"),
            directory: tempfile::tempdir().unwrap(),
        }
    }

    /// 返回原镜像；参数：无；返回：root 资格路径，不构造可信 helper locator。
    pub(super) fn binary(&self) -> &Path {
        &self.binary
    }

    /// 返回另一镜像；参数：无；返回：同源但固定 image ID 为 2 的实际 ELF 路径。
    pub(super) fn replacement(&self) -> &Path {
        &self.replacement
    }

    /// 返回本案隔离目录；参数：无；返回：只用于测试镜像副本的目录。
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
