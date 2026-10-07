//! Git 私有目录删除失败的真实 I/O 回归；仅操作隔离临时夹具。

use super::ProbeLimits;
#[cfg(unix)]
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_private_directory::GitPrivateDirectory;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;
#[cfg(unix)]
use super::{GitSample, sample_git_bounded};
#[cfg(unix)]
use std::path::{Path, PathBuf};

#[test]
fn explicit_completion_removes_private_data_before_success() {
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut private = GitPrivateDirectory::new(&mut probe).unwrap();
    let path = private.path().to_path_buf();
    private
        .write(&path.join("secret"), b"private", &mut probe)
        .unwrap();
    assert_eq!(private.complete(Ok(7)).unwrap(), 7);
    assert!(!path.exists());
    assert_eq!(private.complete(Ok(8)).unwrap(), 8);
}

#[test]
fn moved_private_directory_uses_actual_platform_recovery() {
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut private = GitPrivateDirectory::new(&mut probe).unwrap();
    let path = private.path().to_path_buf();
    let moved = path.with_file_name(format!("diskgraph-git-moved-{}", uuid::Uuid::new_v4()));
    private
        .write(
            &path.join("retained"),
            b"original private payload",
            &mut probe,
        )
        .unwrap();
    #[cfg(not(windows))]
    std::fs::rename(&path, &moved).unwrap();
    #[cfg(windows)]
    rename_original_fixture(&path, &moved, &mut probe).unwrap();
    let observed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(not(windows))]
        {
            let error = private.complete(Ok(9)).unwrap_err();
            assert!(error.contains("original object deletion unconfirmed"));
            let primary = private
                .complete::<()>(Err("original failure".into()))
                .unwrap_err();
            assert!(primary.starts_with("original failure; cleanup also failed:"));
            assert_eq!(
                std::fs::read(moved.join("retained")).unwrap(),
                b"original private payload"
            );
        }
        #[cfg(windows)]
        {
            // 原生句柄可以实际处置同父目录中改名后的原对象；缺少旧名不是完成证明。
            assert_eq!(private.complete(Ok(9)).unwrap(), 9);
            assert!(!path.exists(), "original name unexpectedly exists");
            assert!(!moved.exists(), "actual original object was not deleted");
            assert_eq!(
                private
                    .complete::<()>(Err("original failure".into()))
                    .unwrap_err(),
                "original failure"
            );
            assert_eq!(private.complete(Ok(10)).unwrap(), 10);
            eprintln!("DG_WINDOWS_MOVED_PRIVATE_COMPLETION=1");
        }
    }));
    // 原对象尚未被清理时才恢复原名称，再让同一 owner 显式重试；不按路径直接删除原对象。
    if moved.exists() {
        #[cfg(not(windows))]
        std::fs::rename(&moved, &path).unwrap();
        #[cfg(windows)]
        rename_original_fixture(&moved, &path, &mut probe).unwrap();
    }
    assert_eq!(private.complete(Ok(9)).unwrap(), 9);
    assert!(!path.exists());
    assert!(!moved.exists());
    if let Err(payload) = observed {
        std::panic::resume_unwind(payload);
    }
}

/// 移动隔离夹具的同一目录。参数：原路径、独占目标和原预算；返回：真实改名及身份确认结果。
/// 仅测试准备使用；共享冲突不算成功，不延长期限，不修改产品分享权限。
#[cfg(windows)]
fn rename_original_fixture(
    source: &std::path::Path,
    target: &std::path::Path,
    probe: &mut ProbeBudget,
) -> std::io::Result<()> {
    use super::git_private_allocation::GitPrivateAllocation;
    let identity = GitPrivateAllocation::capture(source).map_err(std::io::Error::other)?;
    loop {
        probe.check().map_err(std::io::Error::other)?;
        let current = GitPrivateAllocation::capture(source).map_err(std::io::Error::other)?;
        if !identity.same_identity(&current) {
            return Err(std::io::Error::other("fixture source identity changed"));
        }
        match std::fs::symlink_metadata(target) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "fixture target exists",
                ));
            }
        }
        match std::fs::rename(source, target) {
            Ok(()) => {
                let moved = GitPrivateAllocation::capture(target).map_err(std::io::Error::other)?;
                if !identity.same_identity(&moved) {
                    return Err(std::io::Error::other("moved fixture identity changed"));
                }
                probe.check().map_err(std::io::Error::other)?;
                return Ok(());
            }
            Err(error) if error.raw_os_error() == Some(32) => {
                // 仅等待外部短期共享冲突；下一轮仍检查原身份、目标和原期限。
                probe.check().map_err(std::io::Error::other)?;
                std::thread::sleep(
                    std::time::Duration::from_millis(20).min(
                        probe
                            .deadline()
                            .saturating_duration_since(std::time::Instant::now()),
                    ),
                );
            }
            Err(error) => return Err(error),
        }
    }
}

#[test]
fn primary_error_is_retained_when_cleanup_succeeds() {
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut private = GitPrivateDirectory::new(&mut probe).unwrap();
    let path = private.path().to_path_buf();
    let error = private
        .complete::<()>(Err("original failure".into()))
        .unwrap_err();
    assert_eq!(error, "original failure");
    assert!(!path.exists());
}

#[cfg(unix)]
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
fn poisoned_public_sample(primary_failure: bool) -> (Result<GitSample, String>, bool) {
    use std::os::unix::fs::PermissionsExt;
    let fixture = GitIsolationFixture::new("sha1");
    let marker = fixture
        .path()
        .parent()
        .unwrap()
        .join("cleanup-private-path");
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let actual = super::git_executable::GitExecutable::resolve(Path::new("git"), &mut probe)
        .unwrap()
        .path()
        .to_path_buf();
    let failure = if primary_failure {
        "printf 'fixture primary failure\\n' >&2; exit 47"
    } else {
        ""
    };
    let script = fixture.script(
        "trusted-cleanup-shim",
        &format!(
            "#!/bin/sh\nif [ \"$4\" = status ]; then\n  printf '%s\\n' \"$GIT_DIR\" > {}\n  mkdir \"$GIT_DIR/blocked\" || exit 48\n  chmod 000 \"$GIT_DIR/blocked\" || exit 49\n  {}\nfi\nexec {} \"$@\"\n",
            quote(&marker),
            failure,
            quote(&actual),
        ),
    );
    let result = sample_git_bounded(&script, fixture.path(), &ProbeLimits::default());
    let private_repo = PathBuf::from(
        std::fs::read_to_string(&marker)
            .expect("trusted shim ran")
            .trim_end(),
    );
    let private_root = private_repo.parent().expect("private repo parent");
    // RED 及 GREEN 都先尝试原生删除，再恢复权限；绝不遗留 chmod000 私有数据。
    let native_cleanup_failed = std::fs::remove_dir_all(private_root).is_err();
    if private_root.exists() {
        let blocked = private_repo.join("blocked");
        if blocked.exists() {
            std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::remove_dir_all(private_root).unwrap();
    }
    (result, native_cleanup_failed)
}

#[cfg(unix)]
#[test]
fn successful_public_sample_refuses_unremoved_private_source() {
    let (result, native_cleanup_failed) = poisoned_public_sample(false);
    assert!(
        native_cleanup_failed,
        "fixture did not cause an actual removal error"
    );
    let error = result.expect_err("sample succeeded while private data remained");
    assert!(error.contains("cleanup"), "{error}");
    assert!(
        error.contains("diskgraph-git-"),
        "missing controlled path: {error}"
    );
}

#[cfg(unix)]
#[test]
fn primary_error_preserves_cleanup_failure_diagnostic() {
    let (result, native_cleanup_failed) = poisoned_public_sample(true);
    assert!(
        native_cleanup_failed,
        "fixture did not cause an actual removal error"
    );
    let error = result.expect_err("primary Git failure was swallowed");
    assert!(error.contains("fixture primary failure"), "{error}");
    assert!(error.contains("cleanup"), "{error}");
    assert!(
        error.contains("diskgraph-git-"),
        "missing controlled path: {error}"
    );
}

#[cfg(windows)]
#[test]
fn open_native_handle_prevents_successful_completion() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut private = GitPrivateDirectory::new(&mut probe).unwrap();
    let path = private.path().join("held");
    private.write(&path, b"private", &mut probe).unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)
        .unwrap();
    let result = private.complete(Ok(7));
    assert!(result.unwrap_err().contains("cleanup"));
    drop(held);
    private.complete(Ok(())).unwrap();
}
