//! 真正 ELF、内核密封与 Atomic 出生的测试材料；不复制生产启动算法。

use super::linux_atomic_launcher_fixture::AtomicLauncherFixture;
use super::linux_atomic_launcher_test_support::{assert_reaped, check, complete, duplicate, line};
use super::linux_scan_image::LinuxScanImage;
use super::{ChildError, ChildSpawnError, ControlWriteStatus};
use crate::{EngineError, ScanWorkerHostConfig};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::Instant;

pub(super) const MFD_EXEC: u32 = 0x0010;
pub(super) const MFD_NOEXEC_SEAL: u32 = 0x0008;
pub(super) const F_SEAL_EXEC: i32 = 0x0020;

/// 唯一拥有本案源名称和真实 A/B ELF 的测试夹具。
/// 来源：原生 Rust PF-06 与 Linux v6.12 memfd/execveat 验收合同。
pub(super) struct ScanImageExecutionFixture {
    fixture: AtomicLauncherFixture,
    end: Instant,
}

impl ScanImageExecutionFixture {
    /// 参数：end 是宿主原绝对期限；返回：显式 artifact 夹具，不查 PATH 或自动编译。
    pub(super) fn new(end: Instant) -> Self {
        Self {
            fixture: AtomicLauncherFixture::new(),
            end,
        }
    }

    /// 参数：无；返回：原 held 源和独立长度/摘要，不从名称或 Hello 自授信任。
    pub(super) fn source(&self) -> (File, ScanWorkerHostConfig) {
        check(self.end).unwrap();
        let path = self.fixture.directory().join("source-image");
        fs::copy(self.fixture.binary(), &path).unwrap();
        let mut file = File::open(&path).unwrap();
        let size = file.metadata().unwrap().len();
        assert!(
            size > 0 && size <= 16 * 1024 * 1024,
            "bounded real ELF fixture"
        );
        let mut hash = Sha256::new();
        let mut block = [0_u8; 65536];
        loop {
            check(self.end).unwrap();
            let count = file.read(&mut block).unwrap();
            if count == 0 {
                break;
            }
            hash.update(&block[..count]);
        }
        let config =
            ScanWorkerHostConfig::from_expected_image(hash.finalize().into(), size).unwrap();
        (file, config)
    }

    /// 参数：flag 为实际内核执行或 NX 旗；返回：同真实 A ELF 的四 seal 对照或原 errno。
    pub(super) fn native_image(&self, flag: u32) -> io::Result<File> {
        let mut image = raw_memfd(flag)?;
        let mut source = File::open(self.fixture.binary())?;
        let length = source.metadata()?.len();
        assert!(length > 0 && length <= 16 * 1024 * 1024);
        let mut block = [0_u8; 65536];
        loop {
            check(self.end).unwrap();
            let count = source.read(&mut block)?;
            if count == 0 {
                break;
            }
            image.write_all(&block[..count])?;
        }
        let result = unsafe { libc::fcntl(image.as_raw_fd(), libc::F_ADD_SEALS, required_seals()) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        assert_seals(&image);
        Ok(image)
    }

    /// 参数：无；返回：原生产 prepare 的真实结果，同 deadline 不延长。
    pub(super) fn prepared(&self) -> Result<LinuxScanImage, EngineError> {
        let (source, config) = self.source();
        LinuxScanImage::prepare(source, &config, self.end, || Ok(()))
    }

    /// 参数：无；返回：运行 B 的真实对照，随后以 B 替换原源名称。
    pub(super) fn replace_with_verified_b(&self) {
        self.execute(File::open(self.fixture.replacement()).unwrap(), "2");
        let replacement = self.fixture.directory().join("replacement");
        fs::copy(self.fixture.replacement(), &replacement).unwrap();
        fs::rename(replacement, self.fixture.directory().join("source-image")).unwrap();
    }

    /// 参数：image 为唯一原 File，expected 为固定 ELF marker；返回：实际双 EOF/pidfd wait 见证。
    pub(super) fn execute(&self, image: File, expected: &str) {
        let launcher = AtomicLauncherFixture::from_file(image, "image", None);
        let mut unwind_owner = None;
        let launched = launcher.spawn(self.end, &mut || check(self.end), &mut unwind_owner);
        assert!(unwind_owner.is_none());
        let mut child = match launched {
            Ok(child) => child,
            Err(failure) => {
                let (error, mut owner) = failure.into_parts();
                let cleanup = owner.as_mut().map(|child| child.cleanup());
                panic!("real sealed exec failed: {error:?}; retained cleanup={cleanup:?}");
            }
        };
        let pidfd = duplicate(&child);
        let observed = catch_unwind(AssertUnwindSafe(|| {
            assert_eq!(
                child.request_control_close().unwrap(),
                ControlWriteStatus::Closed
            );
            let (out, err) = complete(&mut child, self.end);
            assert_eq!(line(&out, "IMAGE="), expected);
            assert_eq!(line(&err, "STDERR_IMAGE="), expected);
            assert_eq!(line(&out, "FIXED="), "sentinel");
            assert_eq!(line(&out, "PATH_ABSENT="), "1");
            assert_eq!(line(&out, "LOADER_ABSENT="), "1");
            assert_eq!(child.exit_code(), Some(0));
            assert!(child.reaped() && child.physically_exited().unwrap());
            assert_reaped(&pidfd);
        }));
        let cleanup = child.cleanup();
        match observed {
            Ok(()) => cleanup.unwrap(),
            Err(payload) => {
                eprintln!("exec qualification cleanup={cleanup:?}");
                resume_unwind(payload);
            }
        }
    }

    /// 参数：image 为真实 NX File；返回：原 execveat EACCES 与清理责任无遗漏的见证。
    pub(super) fn reject_nx(&self, image: File) {
        assert_eq!(image.metadata().unwrap().permissions().mode() & 0o111, 0);
        assert!(unsafe { libc::fcntl(image.as_raw_fd(), libc::F_GET_SEALS) } & F_SEAL_EXEC != 0);
        let launcher = AtomicLauncherFixture::from_file(image, "image", None);
        let mut unwind_owner = None;
        let failed = launcher.spawn(self.end, &mut || check(self.end), &mut unwind_owner);
        assert!(unwind_owner.is_none());
        let failure = match failed {
            Err(failure) => failure,
            Ok(mut child) => {
                let cleanup = child.cleanup();
                panic!("NX ELF unexpectedly executed; cleanup={cleanup:?}");
            }
        };
        let (error, mut owner) = failure.into_parts();
        let retained = owner.is_some();
        let cleanup = owner.as_mut().map(|child| child.cleanup());
        assert!(
            !retained,
            "normal failed-exec cleanup must consume wait: {error:?}; {cleanup:?}"
        );
        match error {
            ChildSpawnError::Operation(ChildError::NativeIo { context, source }) => {
                assert_eq!(context, "atomic child execveat");
                assert_eq!(source.raw_os_error(), Some(libc::EACCES));
            }
            other => panic!("must preserve exact execveat error, got {other:?}"),
        }
    }
}

/// 参数：flag 为精确执行策略旗；返回：一次真实 syscall 的 File 或原错误，不重试降旗。
pub(super) fn raw_memfd(flag: u32) -> io::Result<File> {
    let fd = unsafe {
        libc::memfd_create(
            c"exec-policy-test".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING | flag,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

/// 参数：file 为活的真实 memfd；返回：四个必要 seal 的内核查询断言结果。
pub(super) fn assert_seals(file: &File) {
    let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
    assert!(seals >= 0);
    assert_eq!(seals & required_seals(), required_seals());
}

fn required_seals() -> i32 {
    libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL
}
