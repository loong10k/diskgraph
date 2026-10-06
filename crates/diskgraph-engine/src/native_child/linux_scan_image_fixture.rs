//! 独占真实普通镜像文件；来源：原生 Rust PF-06 Linux sealed image 验收。
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 独立目录与完整测试字节，不执行镜像或伪造内核 seal。
/// 来源：原生 Rust tempfile、File 与 Linux fcntl 合同。
pub(super) struct LinuxScanImageFixture {
    pub(super) directory: tempfile::TempDir,
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
}

impl LinuxScanImageFixture {
    /// 参数：无；返回：512KiB 实际普通文件，足以观察多个64KiB复制边界。
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source-image");
        let bytes: Vec<u8> = (0..512 * 1024).map(|index| (index % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        Self {
            directory,
            path,
            bytes,
        }
    }

    /// 参数：无；返回：夹具准备时独立计算的预期摘要与长度。
    pub(super) fn config(&self) -> ScanWorkerHostConfig {
        ScanWorkerHostConfig::from_expected_image(
            Sha256::digest(&self.bytes).into(),
            self.bytes.len() as u64,
        )
        .unwrap()
    }

    /// 参数：无；返回：原普通文件的新读句柄，不使用显示路径回退。
    pub(super) fn open(&self) -> File {
        File::open(&self.path).unwrap()
    }
}

/// 参数：无；返回：一次测试的原绝对资格期限，调用方不得在准备中刷新。
pub(super) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

/// 参数：result 为真实准备结果；返回：原失败对象，成功则精确失败。
pub(super) fn failure<T>(result: Result<T, EngineError>) -> EngineError {
    match result {
        Ok(_) => panic!("sealed image unexpectedly accepted"),
        Err(error) => error,
    }
}

/// 参数：result 为实际结果、expected 为既有业务类别；返回：无，仅接受该精确类别。
pub(super) fn assert_business<T>(result: Result<T, EngineError>, expected: BusinessError) {
    let actual = failure(result);
    assert!(
        matches!(&actual, EngineError::Business(error) if *error == expected),
        "{actual:?}"
    );
}

/// 参数：file 为已返回实际memfd、expected 为原内容；返回：无，验证内核seals与完整字节。
pub(super) fn assert_sealed_bytes(file: &mut File, expected: &[u8]) {
    // 安全性：只向仍由 File 持有的描述符执行无指针、只读状态查询。
    let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
    assert!(
        seals >= 0,
        "F_GET_SEALS: {}",
        std::io::Error::last_os_error()
    );
    let required = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    assert_eq!(seals & required, required);
    let descriptor_flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    assert!(descriptor_flags >= 0);
    assert_ne!(descriptor_flags & libc::FD_CLOEXEC, 0);
    assert_eq!(file.metadata().unwrap().len(), expected.len() as u64);
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut actual = Vec::new();
    file.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, expected);
}
