//! Windows 原子 root anchor 正控；必须原生执行，不把 Mac cfg 排除或编译当验收。
use super::ProbeLimits;
use super::git_directory_lease::GitDirectoryLease;
use super::git_directory_security::GitDirectorySecurity;
use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use super::probe_execution::{configure_probe_env, run_probe};
use super::windows_git_private_root::WindowsGitPrivateRoot;
use crate::ProbeHost;
use std::ffi::OsStr;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::process::Command;
use std::sync::Arc;

#[test]
fn atomic_root_create_refuses_existing_directory_without_adopting_it() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let lease = GitDirectoryLease::open(&parent, &mut budget).unwrap();
    let security = GitDirectorySecurity::new().unwrap();
    let mut owner = None;
    WindowsGitPrivateRoot::create_into(
        lease.leaf_file(),
        OsStr::new("private"),
        &security,
        &mut owner,
    )
    .unwrap();
    let mut collision = None;
    assert!(
        WindowsGitPrivateRoot::create_into(
            lease.leaf_file(),
            OsStr::new("private"),
            &security,
            &mut collision
        )
        .is_err()
    );
    assert!(
        collision.is_none(),
        "collision must not adopt existing root"
    );
    assert!(
        WindowsGitPrivateRoot::create_into(
            lease.leaf_file(),
            OsStr::new("other"),
            &security,
            &mut owner
        )
        .is_err()
    );
    assert!(
        !parent.join("other").exists(),
        "occupied external owner must reject before native creation"
    );
    let root = owner.as_mut().unwrap();
    root.confirm_created().unwrap();
    let before = GitPrivateAllocation::from_file(root.as_file()).unwrap();
    let after = GitPrivateAllocation::capture(&parent.join("private")).unwrap();
    assert!(before.same_identity(&after));
    for invalid in ["", ".", "..", "a/b", "a\\b", "a:b", "a\0b"] {
        let mut rejected = None;
        assert!(
            WindowsGitPrivateRoot::create_into(
                lease.leaf_file(),
                OsStr::new(invalid),
                &security,
                &mut rejected
            )
            .is_err()
        );
        assert!(rejected.is_none());
    }
}

#[test]
fn moved_root_delete_reopen_targets_original_object_not_foreign_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let lease = GitDirectoryLease::open(&parent, &mut budget).unwrap();
    let mut owner = None;
    WindowsGitPrivateRoot::create_into(
        lease.leaf_file(),
        OsStr::new("original"),
        &GitDirectorySecurity::new().unwrap(),
        &mut owner,
    )
    .unwrap();
    let root = owner.as_mut().unwrap();
    root.confirm_created().unwrap();
    let original = GitPrivateAllocation::from_file(root.as_file()).unwrap();
    // 创建阶段的父目录租约已结束；原 root anchor 继续独占保活，不按旧名恢复。
    drop(lease);
    std::fs::rename(parent.join("original"), parent.join("moved")).unwrap();
    root.confirm_created().unwrap();
    std::fs::create_dir(parent.join("original")).unwrap();
    std::fs::write(parent.join("original/sentinel"), b"foreign").unwrap();
    let moved_identity = GitPrivateAllocation::capture(&parent.join("moved")).unwrap();
    let foreign_identity = GitPrivateAllocation::capture(&parent.join("original")).unwrap();
    let reopened = root.reopen_for_delete().unwrap();
    assert!(original.same_identity(&GitPrivateAllocation::from_file(&reopened).unwrap()));
    assert!(original.same_identity(&moved_identity));
    assert!(!original.same_identity(&foreign_identity));
    assert_eq!(
        std::fs::read(parent.join("original/sentinel")).unwrap(),
        b"foreign"
    );
}

#[test]
fn original_anchor_allows_real_managed_git_cwd_and_unmodified_directory_lease() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let mut budget = Some(ProbeBudget::new(&ProbeLimits::default()).unwrap());
    budget
        .as_mut()
        .unwrap()
        .bind_probe_host(Arc::clone(&host.registry))
        .unwrap();
    let lease = GitDirectoryLease::open(&parent, budget.as_mut().unwrap()).unwrap();
    let mut owner = None;
    WindowsGitPrivateRoot::create_into(
        lease.leaf_file(),
        OsStr::new("git-cwd"),
        &GitDirectorySecurity::new().unwrap(),
        &mut owner,
    )
    .unwrap();
    let root = owner.as_mut().unwrap();
    root.confirm_created().unwrap();
    let path = parent.join("git-cwd");
    let observed = catch_unwind(AssertUnwindSafe(|| {
        // 原 shareREAD 租约仍能打开；不调整生产租约共享标志来让正控通过。
        let child_lease = GitDirectoryLease::open(&path, budget.as_mut().unwrap()).unwrap();
        assert!(
            GitPrivateAllocation::from_file(root.as_file())
                .unwrap()
                .same_identity(&GitPrivateAllocation::from_file(child_lease.leaf_file()).unwrap())
        );
        drop(child_lease);
        for args in [
            vec!["init", "--quiet"],
            vec!["rev-parse", "--is-inside-work-tree"],
        ] {
            let mut command = Command::new("git");
            configure_probe_env(&mut command);
            command.current_dir(&path).args(&args);
            let output = run_probe(&mut command, budget.as_mut().unwrap()).unwrap();
            assert_eq!(output.exit_code, Some(0), "stderr={:?}", output.stderr);
            if args[0] == "rev-parse" {
                assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "true");
            }
        }
        assert!(
            GitPrivateAllocation::from_file(root.as_file())
                .unwrap()
                .same_identity(
                    &GitPrivateAllocation::from_file(&root.reopen_for_delete().unwrap()).unwrap()
                )
        );
    }));
    // 原 session 结束后实际排空同 registry，root/temp 始终保活。旧 drain 可能阻塞。
    drop(budget.take());
    let drained = recovery.drain();
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
    assert!(drained.unwrap());
}

#[test]
fn moved_root_reopen_ignores_junction_replacement_and_preserves_external_target() {
    use super::windows_git_junction_fixture::WindowsGitJunctionFixture;
    let external = tempfile::tempdir().unwrap();
    std::fs::write(external.path().join("sentinel"), b"outside").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let lease = GitDirectoryLease::open(&parent, &mut budget).unwrap();
    let mut owner = None;
    WindowsGitPrivateRoot::create_into(
        lease.leaf_file(),
        OsStr::new("original"),
        &GitDirectorySecurity::new().unwrap(),
        &mut owner,
    )
    .unwrap();
    let root = owner.as_mut().unwrap();
    root.confirm_created().unwrap();
    let original = GitPrivateAllocation::from_file(root.as_file()).unwrap();
    // 创建阶段的父目录租约已结束；原 root anchor 继续独占保活，不按旧名恢复。
    drop(lease);
    std::fs::rename(parent.join("original"), parent.join("moved")).unwrap();
    root.confirm_created().unwrap();
    let mut junction =
        WindowsGitJunctionFixture::create(&parent.join("original"), external.path()).unwrap();
    assert_eq!(
        std::fs::read(parent.join("original/sentinel")).unwrap(),
        b"outside",
        "actual junction must reach controlled external target before the test"
    );
    assert!(
        GitPrivateAllocation::capture(&parent.join("original")).is_err(),
        "no-follow inspection must reject junction"
    );
    let reopened = root.reopen_for_delete().unwrap();
    assert!(original.same_identity(&GitPrivateAllocation::from_file(&reopened).unwrap()));
    assert_eq!(
        std::fs::read(external.path().join("sentinel")).unwrap(),
        b"outside"
    );
    drop(reopened);
    // 正控移除本方junction，验证外部目标原内容仍在；失败时Drop也只remove_dir链接。
    junction.remove().unwrap();
    assert_eq!(
        std::fs::read(external.path().join("sentinel")).unwrap(),
        b"outside"
    );
}

#[test]
fn original_share_read_lease_blocks_delete_reopen_until_released() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let parent_lease = GitDirectoryLease::open(&parent, &mut budget).unwrap();
    let mut owner = None;
    WindowsGitPrivateRoot::create_into(
        parent_lease.leaf_file(),
        OsStr::new("held"),
        &GitDirectorySecurity::new().unwrap(),
        &mut owner,
    )
    .unwrap();
    let root = owner.as_mut().unwrap();
    let child_lease = GitDirectoryLease::open(&parent.join("held"), &mut budget).unwrap();
    // 仅本案的独占空目录做句柄查询、按名对照和可撤销 disposition 探针。
    // 不提升权限，不改变生产方法或原冲突断言。
    emit_granted_access("anchor", root.as_file());
    emit_granted_access("child_lease", child_lease.leaf_file());
    assert!(
        GitPrivateAllocation::from_file(root.as_file())
            .unwrap()
            .same_identity(&GitPrivateAllocation::from_file(child_lease.leaf_file()).unwrap())
    );
    let original = GitPrivateAllocation::from_file(root.as_file()).unwrap();
    emit_name_delete_open(&parent.join("held"), &original);
    let reopened = root.reopen_for_delete();
    if let Ok(file) = &reopened {
        emit_granted_access("delete_reopen", file);
        emit_delete_disposition(file, &original);
    }
    let failure = reopened.unwrap_err();
    assert_eq!(
        failure.raw_os_error(),
        Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
    );
    root.confirm_created().unwrap();
    assert!(
        GitPrivateAllocation::from_file(root.as_file())
            .unwrap()
            .same_identity(&GitPrivateAllocation::from_file(child_lease.leaf_file()).unwrap())
    );
    drop(child_lease);
    let reopened = root.reopen_for_delete().unwrap();
    assert!(
        GitPrivateAllocation::from_file(root.as_file())
            .unwrap()
            .same_identity(&GitPrivateAllocation::from_file(&reopened).unwrap())
    );
}

fn emit_delete_disposition(file: &std::fs::File, original: &GitPrivateAllocation) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
    };
    // 仅已核验完整对象身份的本案空临时目录；不借用用户目录或使用 POSIX/delete-on-close。
    assert!(original.same_identity(&GitPrivateAllocation::from_file(file).unwrap()));
    let mut info = FILE_DISPOSITION_INFO { DeleteFile: true };
    let marked = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&raw const info).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    };
    if marked == 0 {
        let error = std::io::Error::last_os_error();
        eprintln!(
            "DG_ID_DELETE_DISPOSITION success=false error={:?}",
            error.raw_os_error()
        );
        return;
    }
    // 保持原句柄存活，立即撤销 pending 标志，避免把诊断变成实际删除。
    info.DeleteFile = false;
    let cleared = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&raw const info).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    };
    let error = (cleared == 0).then(std::io::Error::last_os_error);
    eprintln!(
        "DG_ID_DELETE_DISPOSITION success=true cleared={} error={:?}",
        cleared != 0,
        error.as_ref().and_then(std::io::Error::raw_os_error)
    );
    assert_ne!(
        cleared, 0,
        "temporary disposition probe must be undone before handle close"
    );
    assert!(original.same_identity(&GitPrivateAllocation::from_file(file).unwrap()));
}

/// 参数：label 为固定诊断标签、file 为原持有对象；返回：无，只记录成功查询的实际权限。
/// 使用 SDK 布局和同步对象查询，不读取文件、不创建新句柄、不替代共享冲突断言。
fn emit_granted_access(label: &str, file: &std::fs::File) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Wdk::Foundation::{NtQueryObject, ObjectBasicInformation};
    use windows_sys::Win32::System::WindowsProgramming::PUBLIC_OBJECT_BASIC_INFORMATION;
    let mut info = PUBLIC_OBJECT_BASIC_INFORMATION::default();
    let mut returned = 0;
    let status = unsafe {
        NtQueryObject(
            file.as_raw_handle(),
            ObjectBasicInformation,
            (&raw mut info).cast(),
            std::mem::size_of_val(&info) as u32,
            &mut returned,
        )
    };
    if status == 0 {
        println!(
            "DG_HANDLE_ACCESS label={label} status=0 returned={returned} granted={:#x} handles={}",
            info.GrantedAccess, info.HandleCount
        );
    } else {
        println!("DG_HANDLE_ACCESS label={label} status={status:#x} returned={returned}");
    }
}

/// 只在私有夹具中按名对照请求同样权限；不作为生产 reopen 或任何路径回退。
fn emit_name_delete_open(path: &std::path::Path, original: &GitPrivateAllocation) {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, SYNCHRONIZE,
    };
    let observed = std::fs::OpenOptions::new()
        .access_mode(DELETE | FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(path);
    match observed {
        Ok(file) => {
            assert!(original.same_identity(&GitPrivateAllocation::from_file(&file).unwrap()));
            eprintln!("DG_NAME_DELETE_OPEN success=true");
            emit_granted_access("name_delete_open", &file);
        }
        Err(error) => eprintln!(
            "DG_NAME_DELETE_OPEN success=false error={:?}",
            error.raw_os_error()
        ),
    }
}

#[test]
fn held_parent_lease_blocks_move_until_creation_phase_is_released() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let lease = GitDirectoryLease::open(&parent, &mut budget).unwrap();
    let mut owner = None;
    WindowsGitPrivateRoot::create_into(
        lease.leaf_file(),
        OsStr::new("original"),
        &GitDirectorySecurity::new().unwrap(),
        &mut owner,
    )
    .unwrap();
    let root = owner.as_mut().unwrap();
    root.confirm_created().unwrap();
    let original = GitPrivateAllocation::from_file(root.as_file()).unwrap();
    let failure = std::fs::rename(parent.join("original"), parent.join("moved")).unwrap_err();
    assert_eq!(
        failure.raw_os_error(),
        Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
    );
    // 只释放创建阶段的父租约；原文件对象 owner 仍持有全部128位身份与anchor。
    drop(lease);
    std::fs::rename(parent.join("original"), parent.join("moved")).unwrap();
    root.confirm_created().unwrap();
    assert!(original.same_identity(&GitPrivateAllocation::capture(&parent.join("moved")).unwrap()));
    let reopened = root.reopen_for_delete().unwrap();
    assert!(original.same_identity(&GitPrivateAllocation::from_file(&reopened).unwrap()));
}
