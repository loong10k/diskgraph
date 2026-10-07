//! D35 scoped 根的新限制；旧可信入口允许这些来源，RED 只证明新边界尚未存在。
//! 本组准备拒绝测试不单独证明“读取前拒绝”；原生句柄读取审计将在实现阶段补充。

use super::ProbeLimits;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_scoped_fixture::{prepare, rejection};
#[cfg(windows)]
use super::git_source_windows_diagnostic::GitSourceWindowsDiagnostic;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;

#[test]
fn explicit_scope_root_never_discovers_an_ancestor_repository() {
    let fixture = GitIsolationFixture::new("sha1");
    let child = fixture.path().join("child-scope");
    std::fs::create_dir(&child).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    rejection(prepare(&child, &mut probe));
}

#[test]
fn explicit_scope_root_rejects_gitfile_to_external_metadata() {
    let fixture = GitIsolationFixture::new("sha1");
    let linked = fixture.sibling_path("linked");
    fixture.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "side",
        linked.to_str().unwrap(),
    ]);
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    rejection(prepare(&linked.canonicalize().unwrap(), &mut probe));
}

#[test]
fn explicit_scope_root_rejects_external_attributes_file() {
    let fixture = GitIsolationFixture::new("sha1");
    let outside = fixture.path().parent().unwrap().join("outside-attributes");
    std::fs::write(&outside, b"tracked text\n").unwrap();
    fixture.git(&["config", "core.attributesFile", outside.to_str().unwrap()]);
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    rejection(prepare(fixture.path(), &mut probe));
}

#[test]
fn explicit_scope_root_rejects_external_excludes_file() {
    let fixture = GitIsolationFixture::new("sha1");
    let outside = fixture.path().parent().unwrap().join("outside-excludes");
    std::fs::write(&outside, b"secret-name\n").unwrap();
    fixture.git(&["config", "core.excludesFile", outside.to_str().unwrap()]);
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    rejection(prepare(fixture.path(), &mut probe));
}

#[test]
fn registered_parent_allows_linked_worktree_and_absolute_common_dependency() {
    let fixture = GitIsolationFixture::new("sha1");
    std::fs::write(fixture.path().join("tracked"), b"stash original\n").unwrap();
    fixture.git(&["stash", "push", "-q"]);
    let linked = fixture.sibling_path("linked");
    fixture.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "side",
        linked.to_str().unwrap(),
    ]);
    let common = fixture.path().join(".git");
    let gitfile = std::fs::read_to_string(linked.join(".git")).unwrap();
    let gitdir = std::path::PathBuf::from(gitfile.trim().strip_prefix("gitdir: ").unwrap());
    std::fs::write(
        gitdir.join("commondir"),
        format!("{}\n", common.to_str().unwrap()),
    )
    .unwrap();
    std::fs::write(linked.join("tracked"), b"linked dirty\n").unwrap();
    let root = fixture.path().parent().unwrap();
    let project =
        diskgraph_core::QualifiedLocator::from_native_path(&linked.canonicalize().unwrap())
            .unwrap();
    #[cfg(windows)]
    let diagnostic = GitSourceWindowsDiagnostic::new();
    let sample = EvidenceProbeSession::new(&ProbeLimits::default())
        .unwrap()
        .sample_git_scoped(std::path::Path::new("git"), root, &project);
    #[cfg(windows)]
    diagnostic.report();
    let sample = sample.unwrap();
    assert_eq!(sample.dirty_count, 1);
    assert_eq!(sample.stash_count, 1);
    assert!(sample.head.is_some());
}

#[test]
fn registered_parent_maps_absolute_attributes_and_excludes_dependencies() {
    let fixture = GitIsolationFixture::new("sha1");
    let root = fixture.path().parent().unwrap();
    let attributes = root.join("scope-attributes");
    let excludes = root.join("scope-excludes");
    std::fs::write(&attributes, b"tracked text\n").unwrap();
    std::fs::write(&excludes, b"ignored-local\n").unwrap();
    fixture.git(&[
        "config",
        "core.attributesFile",
        attributes.to_str().unwrap(),
    ]);
    fixture.git(&["config", "core.excludesFile", excludes.to_str().unwrap()]);
    std::fs::write(fixture.path().join("ignored-local"), b"ignored\n").unwrap();
    let locator = diskgraph_core::QualifiedLocator::from_native_path(fixture.path()).unwrap();
    let before = fixture.metadata();
    let sample = EvidenceProbeSession::new(&ProbeLimits::default())
        .unwrap()
        .sample_git_scoped(std::path::Path::new("git"), root, &locator)
        .unwrap();
    assert_eq!(sample.dirty_count, 0);
    fixture.assert_metadata_unchanged(&before);
}

#[cfg(unix)]
#[test]
fn scoped_source_symbolic_link_is_rejected_without_reading_target_body() {
    let fixture = GitIsolationFixture::new("sha1");
    let outside = fixture.path().parent().unwrap().join("outside-body");
    std::fs::write(&outside, b"outside body\n").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.path().join("linked-body")).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let error = rejection(prepare(fixture.path(), &mut probe));
    assert!(error.contains("unsupported"), "{error}");
    assert_eq!(std::fs::read(outside).unwrap(), b"outside body\n");
}

#[test]
fn nested_untracked_gitfile_cannot_introduce_another_repository_source() {
    let fixture = GitIsolationFixture::new("sha1");
    let nested = fixture.path().join("untracked-repository");
    std::fs::create_dir(&nested).unwrap();
    // 外部 fixture 的真实元数据路径；嵌套根不在本次明确 Git 根契约内。
    let outside = GitIsolationFixture::new("sha1");
    std::fs::write(
        nested.join(".git"),
        format!(
            "gitdir: {}\n",
            outside.path().join(".git").to_str().unwrap()
        ),
    )
    .unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let error = rejection(prepare(fixture.path(), &mut probe));
    assert!(error.contains("unsupported"), "{error}");
}

#[test]
fn renamed_registered_root_invalidates_terminal_capture_with_nested_project() {
    let fixture = GitIsolationFixture::new("sha1");
    let root = fixture.path().parent().unwrap();
    let locator = diskgraph_core::QualifiedLocator::from_native_path(fixture.path()).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let original_root = {
        let (directory, _) =
            super::git_source_directory::GitSourceDirectory::root(root, &mut probe).unwrap();
        directory.version().unwrap()
    };
    let boundary =
        super::git_scope_boundary::GitScopeBoundary::new(root, &locator, &mut probe).unwrap();
    let mut view = super::git_view::GitView::prepare_scoped(
        std::path::Path::new("git"),
        boundary,
        &mut probe,
        super::git_metadata_budget::GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
    .unwrap();
    // moved 始终由另一独立 TempDir 持有，异常展开不会遗留原源文件。
    let holder = tempfile::tempdir().unwrap();
    // 原生身份检查不接受 macOS /var 链接别名；先规范化已存在的独占夹具父目录。
    let moved = holder.path().canonicalize().unwrap().join("moved-scope");
    // 源目录能力允许共享删除；不能把私有目录 no-delete 租约的性质套用于来源。
    // 验收真实 OS 结果：阻止名称变更则保持捕获，允许变更则必须拒绝终态稳定声明。
    let changed = match std::fs::rename(root, &moved) {
        Ok(()) => {
            assert!(!root.exists());
            assert!(moved.is_dir());
            true
        }
        Err(error) if cfg!(windows) && error.raw_os_error() == Some(32) => {
            assert!(root.is_dir());
            assert!(!moved.exists());
            false
        }
        Err(error) => panic!("actual root rename failed unexpectedly: {error}"),
    };
    let output = view.run(&super::git_scoped_fixture::STATUS[1..], &mut probe);
    let terminal = view.verify(&mut probe);
    let terminal = view.complete(terminal);
    // 原 view 仍持有来源子目录/文件能力；Windows 不保证这些句柄允许父目录再次改名。
    // 先完成原私有目录处置并释放原捕获，再恢复隔离源名称，不改变终态验证结果。
    drop(view);
    drop(probe);
    if changed {
        // 夹具恢复的期限不进入原业务预算，也不重建探针池。
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        restore_fixture_root(&moved, root, &original_root, until, || {});
    }
    let output = output.unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(
        output.stdout.is_empty(),
        "private command must survive source root rename"
    );
    if changed {
        assert!(
            terminal.is_err(),
            "registered root namespace changed but capture was called stable"
        );
    } else {
        assert!(
            terminal.is_ok(),
            "blocked root replacement must retain the original verified capture: {terminal:?}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn registered_root_open_restores_thread_hydration_policy_on_success_and_error() {
    unsafe extern "C" {
        fn getiopolicy_np(iotype: i32, scope: i32) -> i32;
    }
    let fixture = GitIsolationFixture::new("sha1");
    let root = fixture.path();
    let locator = diskgraph_core::QualifiedLocator::from_native_path(root).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let previous = unsafe { getiopolicy_np(3, 1) };
    assert!(previous >= 0);
    let opened = super::git_scope_boundary::GitScopeBoundary::new(root, &locator, &mut probe);
    assert!(opened.is_ok());
    assert_eq!(unsafe { getiopolicy_np(3, 1) }, previous);
    let missing = root.join("missing-root");
    let locator = diskgraph_core::QualifiedLocator::from_native_path(&missing).unwrap();
    assert!(
        super::git_scope_boundary::GitScopeBoundary::new(&missing, &locator, &mut probe).is_err()
    );
    assert_eq!(unsafe { getiopolicy_np(3, 1) }, previous);
}

#[test]
fn in_scope_gitdir_does_not_authorize_an_external_common_directory() {
    let fixture = GitIsolationFixture::new("sha1");
    let linked = fixture.sibling_path("linked");
    fixture.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "side",
        linked.to_str().unwrap(),
    ]);
    let gitfile = std::fs::read_to_string(linked.join(".git")).unwrap();
    let original = std::path::PathBuf::from(gitfile.trim().strip_prefix("gitdir: ").unwrap());
    let in_scope = linked.join("metadata");
    std::fs::rename(&original, &in_scope).unwrap();
    std::fs::write(linked.join(".git"), b"gitdir: metadata\n").unwrap();
    std::fs::write(
        in_scope.join("commondir"),
        format!("{}\n", fixture.path().join(".git").to_str().unwrap()),
    )
    .unwrap();
    // 旧可信入口的合法外部依赖作为正控制；不把配置损坏产生的错误当 scoped 门禁。
    let control = sample_git_bounded(
        std::path::Path::new("git"),
        &linked,
        &ProbeLimits::default(),
    )
    .unwrap();
    assert!(control.head.is_some());
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let error = rejection(prepare(&linked.canonicalize().unwrap(), &mut probe));
    assert!(error.contains("outside authorized scope"), "{error}");
}

#[test]
fn replaced_registered_root_ancestor_rejects_terminal_without_reopening_source() {
    let fixture = GitIsolationFixture::new("sha1");
    let outer = tempfile::tempdir().unwrap();
    let container = outer.path().join("container");
    let root = container.join("scope");
    let project = root.join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::rename(fixture.path(), &project).unwrap();
    let root = root.canonicalize().unwrap();
    let project = project.canonicalize().unwrap();
    let locator = diskgraph_core::QualifiedLocator::from_native_path(&project).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let boundary =
        super::git_scope_boundary::GitScopeBoundary::new(&root, &locator, &mut probe).unwrap();
    let mut view = super::git_view::GitView::prepare_scoped(
        std::path::Path::new("git"),
        boundary,
        &mut probe,
        super::git_metadata_budget::GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
    .unwrap();
    let moved = outer.path().join("original-container");
    let replacement = std::fs::rename(&container, &moved);
    #[cfg(windows)]
    if let Err(error) = &replacement {
        // Windows 可在持有子目录能力时直接禁止祖先替换；必须验证拒绝及释放后的恢复。
        assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
        eprintln!(
            "ancestor replacement blocked by Windows: {:?}",
            error.raw_os_error()
        );
        assert!(container.is_dir());
        assert!(!moved.exists());
        assert_eq!(std::fs::read(project.join("tracked")).unwrap(), b"AAAA\n");
        let output = view.run(&super::git_scoped_fixture::STATUS[1..], &mut probe);
        let terminal = view.verify(&mut probe);
        let output = view.complete(output).unwrap();
        terminal.expect("blocked ancestor replacement preserves the original source identity");
        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.is_empty());
        // 关闭 owner 后必须解除本次持有的限制，不能靠句柄泄漏令攻击测试通过。
        drop(view);
        std::fs::rename(&container, &moved).expect("capture cleanup releases the route");
        std::fs::rename(&moved, &container).unwrap();
        return;
    }
    replacement.unwrap();
    std::fs::create_dir_all(container.join("scope/project")).unwrap();
    std::fs::write(
        container.join("scope/project/tracked"),
        b"foreign replacement\n",
    )
    .unwrap();
    let output = view.run(&super::git_scoped_fixture::STATUS[1..], &mut probe);
    let error = view.verify(&mut probe).unwrap_err();
    assert!(error.contains("registered root route changed"), "{error}");
    let terminal = view.complete::<()>(Err(error));
    let output = output.unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(
        output.stdout.is_empty(),
        "private output must not use the replacement ancestor route"
    );
    let error = terminal.unwrap_err();
    assert!(error.contains("registered root route changed"), "{error}");
}

#[test]
fn unrelated_ancestor_sibling_activity_keeps_registered_route_valid() {
    let fixture = GitIsolationFixture::new("sha1");
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = super::git_scoped_fixture::prepare(fixture.path(), &mut probe).unwrap();
    let sibling = fixture.path().parent().unwrap().join("unrelated-sibling");
    std::fs::create_dir(&sibling).unwrap();
    std::fs::write(sibling.join("activity"), b"not a Git input").unwrap();
    let output = view.run(super::git_scoped_fixture::STATUS, &mut probe);
    let verified = view.verify(&mut probe);
    let output = view.complete(output).unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(output.stdout.is_empty());
    verified.expect("ancestor identity is stable despite sibling directory activity");
}

#[cfg(not(windows))]
use super::EvidenceProbeSession;
#[cfg(windows)]
use super::native_evidence_test_session::NativeEvidenceTestSession as EvidenceProbeSession;

#[cfg(windows)]
use super::native_evidence_test_session::sample_git_bounded;
#[cfg(not(windows))]
use super::sample_git_bounded;

// 仅在已完成的隔离测试中恢复同一源对象；不作为产品文件操作或请求续期入口。
fn restore_fixture_root(
    moved: &std::path::Path,
    root: &std::path::Path,
    original_root: &super::git_directory_version::GitDirectoryVersion,
    until: std::time::Instant,
    mut on_sharing: impl FnMut(),
) -> usize {
    let mut restoration =
        super::probe_budget::ProbeBudget::until(&ProbeLimits::default(), until).unwrap();
    let mut retries = 0;
    loop {
        restoration.check().unwrap();
        let current = {
            let (directory, _) =
                super::git_source_directory::GitSourceDirectory::root(moved, &mut restoration)
                    .unwrap();
            directory.version().unwrap()
        };
        assert!(
            original_root.same_identity(&current),
            "fixture restoration must retain the original directory"
        );
        assert_eq!(
            std::fs::symlink_metadata(root).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        match std::fs::rename(moved, root) {
            Ok(()) => break,
            Err(error) if cfg!(windows) && error.raw_os_error() == Some(32) => {
                if retries == 0 {
                    eprintln!("DG_FIXTURE_ROOT_RESTORE_SHARING_RETAINED=32");
                }
                retries += 1;
                on_sharing();
                restoration.check().expect("original fixture sharing conflict did not clear under fixed restoration deadline");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(error) => panic!("original fixture restoration failed: {error}"),
        }
    }
    let restored = {
        let (directory, _) =
            super::git_source_directory::GitSourceDirectory::root(root, &mut restoration).unwrap();
        directory.version().unwrap()
    };
    assert!(original_root.same_identity(&restored));
    retries
}

#[cfg(windows)]
#[test]
fn original_fixture_restore_retries_actual_no_delete_directory_handle() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let holder = tempfile::tempdir().unwrap();
    let parent = holder.path().canonicalize().unwrap();
    let root = parent.join("original");
    let moved = parent.join("moved");
    std::fs::create_dir(&root).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut observation =
        super::probe_budget::ProbeBudget::until(&ProbeLimits::default(), until).unwrap();
    let original = {
        let (directory, _) =
            super::git_source_directory::GitSourceDirectory::root(&root, &mut observation).unwrap();
        directory.version().unwrap()
    };
    restore_fixture_root(&root, &moved, &original, until, || {});
    let blocker = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(&moved)
        .unwrap();
    // 原生 no-delete 句柄只在已观察到实际 OS32 后释放，调度延迟不能跳过负向控制。
    let (release, waiting) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            waiting
                .recv_timeout(until.saturating_duration_since(std::time::Instant::now()))
                .unwrap();
            drop(blocker);
        });
        let mut released = false;
        let retries = restore_fixture_root(&moved, &root, &original, until, || {
            if !released {
                release.send(()).unwrap();
                released = true;
            }
        });
        assert!(retries > 0);
    });
}
