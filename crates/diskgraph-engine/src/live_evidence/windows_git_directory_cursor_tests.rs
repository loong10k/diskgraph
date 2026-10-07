//! 原生目录枚举及记录解码边界；不代替清理、产品启用或严格RSS验收。
use super::ProbeLimits;
use super::git_directory_lease::GitDirectoryLease;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use super::windows_git_directory_cursor::{WindowsGitDirectoryCursor, parse_entry};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows_sys::Win32::Storage::FileSystem::FILE_ID_EXTD_DIR_INFO;

#[test]
fn held_directory_names_ignore_foreign_path_argument() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let original = parent.join("original");
    let foreign = parent.join("foreign");
    std::fs::create_dir(&original).unwrap();
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(original.join("owned"), b"original").unwrap();
    std::fs::write(foreign.join("sentinel"), b"foreign").unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut lease = GitDirectoryLease::open(&original, &mut probe).unwrap();
    let mut metadata = GitMetadataBudget::default();
    println!("DG_HELD_DIRECTORY_ARGUMENT_CONTROL=1");
    let names = lease
        .read_names(&foreign, &mut metadata, &mut probe)
        .unwrap();
    assert_eq!(
        names,
        [OsString::from("owned")],
        "held directory must never enumerate the foreign argument path"
    );
    assert_eq!(std::fs::read(foreign.join("sentinel")).unwrap(), b"foreign");
}

#[test]
fn held_directory_pages_preserve_raw_utf16_and_full_file_ids() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let mut expected = BTreeSet::new();
    for index in 0..1200 {
        let name = OsString::from(format!("entry_{index:04}"));
        std::fs::write(parent.join(&name), b"x").unwrap();
        expected.insert(name);
    }
    // Windows 原生名称可含未配对UTF-16单元，不进行有损UTF-8转换。
    let raw = OsString::from_wide(&[110, 0xd800]);
    std::fs::write(parent.join(&raw), b"raw").unwrap();
    expected.insert(raw);
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut lease = GitDirectoryLease::open(&parent, &mut probe).unwrap();
    let mut metadata = GitMetadataBudget::default();
    let names = lease
        .read_names(&parent, &mut metadata, &mut probe)
        .unwrap();
    assert_eq!(names.into_iter().collect::<BTreeSet<_>>(), expected);
    assert_eq!(metadata.remaining_entries(), 32_768 - expected.len());
    let mut cursor =
        WindowsGitDirectoryCursor::new(lease.leaf_file().try_clone().unwrap()).unwrap();
    let mut observed = BTreeSet::new();
    while let Some((name, id, _)) = cursor.next_entry(&mut probe).unwrap() {
        let allocation = GitPrivateAllocation::capture(&parent.join(&name)).unwrap();
        let descriptor = allocation.windows_file_id_descriptor();
        assert_eq!(id, unsafe {
            descriptor.Anonymous.ExtendedFileId.Identifier
        });
        assert!(
            observed.insert(name),
            "duplicate record across native pages"
        );
    }
    assert_eq!(observed, expected);
}

#[test]
fn directory_record_decoder_rejects_invalid_lengths_offsets_and_components() {
    let mut record = FILE_ID_EXTD_DIR_INFO {
        FileNameLength: 2,
        FileName: [120],
        FileId: windows_sys::Win32::Storage::FileSystem::FILE_ID_128 {
            Identifier: [1; 16],
        },
        ..FILE_ID_EXTD_DIR_INFO::default()
    };
    let encode = |record: &FILE_ID_EXTD_DIR_INFO| {
        let mut page = vec![0u8; 256];
        unsafe { std::ptr::write_unaligned(page.as_mut_ptr().cast(), *record) };
        page
    };
    assert_eq!(
        parse_entry(&encode(&record), 0).unwrap().0,
        OsString::from("x")
    );
    for length in [0, 1, 65534, u32::MAX] {
        let mut invalid = record;
        invalid.FileNameLength = length;
        assert!(parse_entry(&encode(&invalid), 0).is_err());
    }
    for offset in [1, 8, 88, 256, u32::MAX] {
        let mut invalid = record;
        invalid.NextEntryOffset = offset;
        assert!(parse_entry(&encode(&invalid), 0).is_err());
    }
    for unit in [0, 47, 58, 92] {
        let mut invalid = record;
        invalid.FileName = [unit];
        assert!(parse_entry(&encode(&invalid), 0).is_err());
    }
    record.FileId.Identifier = [0; 16];
    assert!(parse_entry(&encode(&record), 0).is_err());
    assert!(parse_entry(&[], 0).is_err());
    assert!(parse_entry(&[0; 256], usize::MAX).is_err());
}

// 单次独占空根及真实父句柄；所有移动/联接均只作用于本案临时目录。
fn cleanup_fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    super::windows_git_private_root::WindowsGitPrivateRoot,
    WindowsGitDirectoryCursor,
    ProbeBudget,
) {
    use super::git_directory_security::GitDirectorySecurity;
    use super::windows_git_private_root::WindowsGitPrivateRoot;
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let lease = GitDirectoryLease::open(&parent, &mut probe).unwrap();
    let mut root = None;
    WindowsGitPrivateRoot::create_into(
        lease.leaf_file(),
        std::ffi::OsStr::new("original"),
        &GitDirectorySecurity::new().unwrap(),
        &mut root,
    )
    .unwrap();
    // 创建阶段已结束；原root继续保活，shareREAD父lease不能阻止本案移动正控。
    drop(lease);
    let root = root.unwrap();
    let cursor = WindowsGitDirectoryCursor::new(root.as_file().try_clone().unwrap()).unwrap();
    (temp, parent, root, cursor, probe)
}

#[test]
fn verified_cleanup_child_uses_original_parent_after_move_and_foreign_root_replacement() {
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    std::fs::write(parent.join("original/owned"), b"original").unwrap();
    let (name, id, _) = cursor.next_entry(&mut probe).unwrap().unwrap();
    std::fs::rename(parent.join("original"), parent.join("moved")).unwrap();
    std::fs::create_dir(parent.join("original")).unwrap();
    std::fs::write(parent.join("original/owned"), b"foreign").unwrap();
    let file = cursor
        .open_verified_child(&name, id, false, &mut probe)
        .unwrap();
    assert!(
        GitPrivateAllocation::from_file(&file)
            .unwrap()
            .windows_matches_file_id(&id)
    );
    assert_eq!(
        std::fs::read(parent.join("moved/owned")).unwrap(),
        b"original"
    );
    assert_eq!(
        std::fs::read(parent.join("original/owned")).unwrap(),
        b"foreign"
    );
    // 删除只作用于已核完整ID的本案句柄；关闭后检查实际名称消失及陌生根哨兵。
    mark_fixture_delete(&file);
    drop(file);
    assert_fixture_absent(&parent.join("moved/owned"));
    assert_eq!(
        std::fs::read(parent.join("original/owned")).unwrap(),
        b"foreign"
    );
    let delete_root = _root.reopen_for_delete().unwrap();
    mark_fixture_delete(&delete_root);
    // pending并非回收完成，释放所有原目录句柄后才检查最终效果。
    drop(delete_root);
    drop(cursor);
    drop(_root);
    assert_fixture_absent(&parent.join("moved"));
    assert_eq!(
        std::fs::read(parent.join("original/owned")).unwrap(),
        b"foreign"
    );
    println!("DG_VERIFIED_CHILD_AND_ROOT_LAST_CLOSE_DELETE=1");
}

// 参数：本案原对象路径；返回：无，只接受真实不存在，不将权限拒绝/pending错误当消失。
fn assert_fixture_absent(path: &std::path::Path) {
    let error = std::fs::symlink_metadata(path).expect_err("original object still exists");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(
        matches!(error.raw_os_error(), Some(2 | 3)),
        "unexpected absence error: {error}"
    );
}

// 参数：本案独占、已核身份的DELETE句柄；返回：无，原生失败必须使测试失败。
fn mark_fixture_delete(file: &std::fs::File) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
    };
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&raw const info).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    };
    assert_ne!(
        result,
        0,
        "actual fixture deletion failed: {}",
        std::io::Error::last_os_error()
    );
}

#[test]
fn verified_cleanup_child_rejects_same_name_foreign_replacement_and_type_mismatch() {
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let path = parent.join("original/owned");
    std::fs::write(&path, b"original").unwrap();
    let (name, id, _) = cursor.next_entry(&mut probe).unwrap().unwrap();
    assert!(
        cursor
            .open_verified_child(&name, id, true, &mut probe)
            .is_err()
    );
    std::fs::rename(&path, parent.join("original/moved")).unwrap();
    std::fs::write(&path, b"foreign").unwrap();
    let error = cursor
        .open_verified_child(&name, id, false, &mut probe)
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign");
    assert_eq!(
        std::fs::read(parent.join("original/moved")).unwrap(),
        b"original"
    );
    for invalid in ["", ".", "..", "a/b", "a\\b", "a:b", "a\0b"] {
        assert_eq!(
            cursor
                .open_verified_child(std::ffi::OsStr::new(invalid), id, false, &mut probe)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
    assert_eq!(
        cursor
            .open_verified_child(&name, [0; 16], false, &mut probe)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
}

#[test]
fn verified_cleanup_child_refuses_real_junction_without_touching_external_target() {
    use super::windows_git_junction_fixture::WindowsGitJunctionFixture;
    let external = tempfile::tempdir().unwrap();
    std::fs::write(external.path().join("sentinel"), b"external").unwrap();
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let path = parent.join("original/child");
    std::fs::create_dir(&path).unwrap();
    let (name, id, _) = cursor.next_entry(&mut probe).unwrap().unwrap();
    std::fs::rename(&path, parent.join("original/moved")).unwrap();
    let mut junction = WindowsGitJunctionFixture::create(&path, external.path()).unwrap();
    assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"external");
    assert!(
        cursor
            .open_verified_child(&name, id, true, &mut probe)
            .is_err()
    );
    assert_eq!(
        std::fs::read(external.path().join("sentinel")).unwrap(),
        b"external"
    );
    junction.remove().unwrap();
}

#[test]
fn verified_cleanup_child_preserves_real_share_read_conflict_and_retries_same_id() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let path = parent.join("original/owned");
    std::fs::write(&path, b"original").unwrap();
    let (name, id, _) = cursor.next_entry(&mut probe).unwrap().unwrap();
    let lease = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)
        .unwrap();
    assert_eq!(
        cursor
            .open_verified_child(&name, id, false, &mut probe)
            .unwrap_err()
            .raw_os_error(),
        Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
    );
    drop(lease);
    let file = cursor
        .open_verified_child(&name, id, false, &mut probe)
        .unwrap();
    assert!(
        GitPrivateAllocation::from_file(&file)
            .unwrap()
            .windows_matches_file_id(&id)
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
}

#[test]
fn pending_original_deletion_is_not_complete_until_external_handle_closes() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let (_temp, parent, root, cursor, mut probe) = cleanup_fixture();
    let parent_lease = GitDirectoryLease::open(&parent, &mut probe).unwrap();
    let parent_identity = GitPrivateAllocation::from_file(parent_lease.leaf_file()).unwrap();
    // 仅夹具的固定temp父目录：先核原身份，再保留shareALL卷提示，释放创建/捕获shareREAD租约。
    // 此路径打开不属于产品清理或恢复接口，也不放宽原共享租约冲突测试。
    let hint = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(&parent)
        .unwrap();
    assert!(parent_identity.same_identity(&GitPrivateAllocation::from_file(&hint).unwrap()));
    drop(parent_lease);
    let identity = GitPrivateAllocation::from_file(root.as_file()).unwrap();
    let original = parent.join("original");
    let moved = parent.join("moved");
    std::fs::rename(&original, &moved).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join("foreign"), b"retain foreign root").unwrap();
    let external = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(&moved)
        .unwrap();
    assert!(identity.same_identity(&GitPrivateAllocation::from_file(&external).unwrap()));
    compare_native_sdk_file_id(&hint, &identity, "present");
    assert!(
        !super::windows_git_deletion_witness::WindowsGitDeletionWitness::confirm_absent(
            &hint, &identity, &mut probe,
        )
        .unwrap()
    );
    let mut observation = None;
    super::windows_git_removal_observation::WindowsGitRemovalObservation::prepare_into(
        &hint,
        &identity,
        &mut probe,
        &mut observation,
    )
    .unwrap();
    let delete = root.reopen_for_delete().unwrap();
    mark_fixture_delete(&delete);
    drop(delete);
    drop(cursor);
    drop(root);
    compare_native_sdk_file_id(&hint, &identity, "pending");
    // 外部原句柄仍存活时没有完整原 REMOVE 通知；保持 Pending，不释放清理责任。
    assert!(
        !observation
            .as_mut()
            .unwrap()
            .confirm(&identity, &mut probe)
            .unwrap(),
        "held original object must remain pending"
    );
    assert_eq!(
        std::fs::read(original.join("foreign")).unwrap(),
        b"retain foreign root"
    );
    drop(external);
    compare_native_sdk_file_id(&hint, &identity, "closed");
    // 通知投递可晚于 close；沿用原 probe 预算，绝不以路径缺失或 ID 错误确认完成。
    while !observation
        .as_mut()
        .unwrap()
        .confirm(&identity, &mut probe)
        .unwrap()
    {
        probe.check().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_fixture_absent(&moved);
    assert_eq!(
        std::fs::read(original.join("foreign")).unwrap(),
        b"retain foreign root"
    );
    println!("DG_ORIGINAL_ID_PENDING_THEN_ABSENT=1");
}

// 原生验收对照：三阶段使用同一个已核验原身份，不能作为生产失败后的格式回退。
fn compare_native_sdk_file_id(hint: &std::fs::File, identity: &GitPrivateAllocation, phase: &str) {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdType, OpenFileById,
    };
    let extended = identity.windows_file_id_descriptor();
    let id = unsafe { extended.Anonymous.ExtendedFileId.Identifier };
    let protocol =
        super::windows_git_native_id_protocol::WindowsGitNativeIdProtocol::from_hint(hint).unwrap();
    assert_eq!(
        protocol.byte_length(&id).unwrap(),
        8,
        "NTFS-only diagnostic"
    );
    let descriptor = FILE_ID_DESCRIPTOR {
        dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: FileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 {
            FileId: i64::from_ne_bytes(id[..8].try_into().unwrap()),
        },
    };
    let handle = unsafe {
        OpenFileById(
            hint.as_raw_handle(),
            &descriptor,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        let error = std::io::Error::last_os_error();
        eprintln!(
            "DG_SDK_FILE_ID_PHASE={phase}; win32={:?}",
            error.raw_os_error()
        );
        assert_ne!(
            phase, "present",
            "original SDK identity positive control: {error}"
        );
    } else {
        let file = unsafe { std::fs::File::from_raw_handle(handle) };
        assert!(identity.same_identity(&GitPrivateAllocation::from_file(&file).unwrap()));
        eprintln!("DG_SDK_FILE_ID_PHASE={phase}; same_full_identity=1");
        assert_eq!(
            phase, "present",
            "original object must not reopen after deletion mark"
        );
    }
}

#[test]
fn cleanup_cursor_retains_current_child_across_open_failure_and_pending_deletion() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    std::fs::write(parent.join("original/first"), b"first").unwrap();
    std::fs::write(parent.join("original/second"), b"second").unwrap();
    let parent_label = parent.join("original");
    let mut capacity = super::git_private_capacity::GitPrivateCapacity::new(
        &parent_label,
        128 << 20,
        0,
        &mut probe,
    )
    .unwrap();
    for name in ["first", "second"] {
        let path = parent_label.join(name);
        let file = std::fs::File::open(&path).unwrap();
        capacity.observe(&path, &file, &mut probe).unwrap();
    }
    let (initial, name, _) = cursor
        .open_next_cleanup_child(&parent_label, &capacity, &mut probe)
        .unwrap()
        .unwrap();
    let identity = GitPrivateAllocation::from_file(&initial).unwrap();
    drop(initial);
    assert!(
        cursor.next_entry(&mut probe).is_err(),
        "unconfirmed child cannot be skipped"
    );
    assert!(!cursor.confirm_cleanup_child_absent(&mut probe).unwrap());
    let path = parent.join("original").join(&name);
    let blocker = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)
        .unwrap();
    assert_eq!(
        cursor
            .open_next_cleanup_child(&parent_label, &capacity, &mut probe)
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
    assert!(cursor.next_entry(&mut probe).is_err());
    drop(blocker);
    let external = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    let (file, retried_name, _) = cursor
        .open_next_cleanup_child(&parent_label, &capacity, &mut probe)
        .unwrap()
        .unwrap();
    assert_eq!(retried_name, name);
    assert!(identity.same_identity(&GitPrivateAllocation::from_file(&file).unwrap()));
    cursor
        .mark_cleanup_child(&file, &parent_label, &capacity, &mut probe)
        .unwrap();
    drop(file);
    assert_eq!(
        cursor
            .confirm_cleanup_child_absent(&mut probe)
            .unwrap_err()
            .raw_os_error(),
        Some(5)
    );
    assert!(
        cursor.next_entry(&mut probe).is_err(),
        "pending deletion cannot advance the cursor"
    );
    drop(external);
    assert!(cursor.confirm_cleanup_child_absent(&mut probe).unwrap());
    assert_fixture_absent(&path);
    let (second, second_name, _) = cursor
        .open_next_cleanup_child(&parent_label, &capacity, &mut probe)
        .unwrap()
        .unwrap();
    assert_ne!(
        second_name, name,
        "only final confirmation permits next child"
    );
    cursor
        .mark_cleanup_child(&second, &parent_label, &capacity, &mut probe)
        .unwrap();
    drop(second);
    assert!(cursor.confirm_cleanup_child_absent(&mut probe).unwrap());
    assert!(
        cursor
            .open_next_cleanup_child(&parent_label, &capacity, &mut probe)
            .unwrap()
            .is_none()
    );
    println!("DG_CLEANUP_CURSOR_RETRY_SAME_CHILD_THEN_ADVANCE=1");
}

#[test]
fn native_id_protocol_refuses_loss_and_preserves_refs_full_identity() {
    use super::windows_git_native_id_protocol::WindowsGitNativeIdProtocol;
    let ntfs = WindowsGitNativeIdProtocol::NtfsFileReference;
    let refs = WindowsGitNativeIdProtocol::RefsExtendedFileId;
    let mut id = [0u8; 16];
    assert!(ntfs.byte_length(&id).is_err());
    assert!(refs.byte_length(&id).is_err());
    id[0] = 1;
    assert_eq!(ntfs.byte_length(&id).unwrap(), 8);
    assert_eq!(refs.byte_length(&id).unwrap(), 16);
    for index in 8..16 {
        id[index] = 1;
        assert!(
            ntfs.byte_length(&id).is_err(),
            "nonzero identity bits must never be dropped"
        );
        assert_eq!(refs.byte_length(&id).unwrap(), 16);
        id[index] = 0;
    }
    // 此案只验证格式边界，不代替ReFS卷上的原生文件系统验收。
    println!("DG_NATIVE_ID_FORMAT_LOSS_REFUSED=1");
}

#[test]
fn cleanup_child_requires_original_owner_registration_and_rejects_foreign_replacement() {
    for registered in [false, true] {
        let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
        let label = parent.join("original");
        let path = label.join("owned");
        let mut capacity =
            super::git_private_capacity::GitPrivateCapacity::new(&label, 128 << 20, 0, &mut probe)
                .unwrap();
        std::fs::write(&path, b"original").unwrap();
        if registered {
            let file = std::fs::File::open(&path).unwrap();
            capacity.observe(&path, &file, &mut probe).unwrap();
            drop(file);
            std::fs::rename(&path, parent.join("retained")).unwrap();
            std::fs::write(&path, b"foreign").unwrap();
        }
        let error = cursor
            .open_next_cleanup_child(&label, &capacity, &mut probe)
            .unwrap_err();
        if !registered {
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
            assert!(cursor.next_entry(&mut probe).is_err());
            println!("DG_ORIGINAL_FOREIGN_REGISTRATION_RED_READY=1");
        }
        assert!(
            error.to_string().contains(if registered {
                "registered identity changed"
            } else {
                "not registered"
            }),
            "private Git first foreign error must preserve registration rejection: {error}"
        );
        assert!(
            cursor.next_entry(&mut probe).is_err(),
            "unverified child cannot be skipped"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            if registered {
                b"foreign".as_slice()
            } else {
                b"original".as_slice()
            }
        );
        if registered {
            assert_eq!(std::fs::read(parent.join("retained")).unwrap(), b"original");
        }
    }
    println!("DG_CLEANUP_REQUIRES_ORIGINAL_OWNER_LEDGER=1");
}

#[test]
fn cleanup_mark_requires_current_identity_and_live_budget_before_mutation() {
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let label = parent.join("original");
    let path = label.join("owned");
    std::fs::write(&path, b"original").unwrap();
    std::fs::write(parent.join("foreign"), b"retain foreign").unwrap();
    let mut capacity =
        super::git_private_capacity::GitPrivateCapacity::new(&label, 128 << 20, 0, &mut probe)
            .unwrap();
    let registered = std::fs::File::open(&path).unwrap();
    capacity.observe(&path, &registered, &mut probe).unwrap();
    drop(registered);
    let (file, _, _) = cursor
        .open_next_cleanup_child(&label, &capacity, &mut probe)
        .unwrap()
        .unwrap();
    let foreign = std::fs::File::open(parent.join("foreign")).unwrap();
    assert!(
        cursor
            .mark_cleanup_child(&foreign, &label, &capacity, &mut probe)
            .is_err()
    );
    assert!(!cursor.cleanup_child_delete_requested());
    let mut expired = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    expired.expire_for_test();
    assert!(
        cursor
            .mark_cleanup_child(&file, &label, &capacity, &mut expired)
            .is_err()
    );
    assert!(!cursor.cleanup_child_delete_requested());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(
        std::fs::read(parent.join("foreign")).unwrap(),
        b"retain foreign"
    );
    cursor
        .mark_cleanup_child(&file, &label, &capacity, &mut probe)
        .unwrap();
    assert!(cursor.cleanup_child_delete_requested());
    assert!(
        cursor
            .open_next_cleanup_child(&label, &capacity, &mut probe)
            .is_err()
    );
    assert!(cursor.next_entry(&mut probe).is_err());
    assert_eq!(
        cursor
            .confirm_cleanup_child_absent(&mut probe)
            .unwrap_err()
            .raw_os_error(),
        Some(5)
    );
    drop(file);
    println!("DG_CLEANUP_MARK_IDENTITY_AND_BUDGET_GUARDS=1");
}

#[test]
fn removal_notification_decoder_refuses_loss_and_partial_bad_pages() {
    use super::windows_git_removal_observation::matches_removal;
    let mut child = [0u8; 16];
    child[0] = 1;
    let mut parent = [0u8; 16];
    parent[0] = 2;
    let mut record = vec![0u8; 88];
    record[4..8].copy_from_slice(&2u32.to_le_bytes());
    record[64..72].copy_from_slice(&child[..8]);
    record[72..80].copy_from_slice(&parent[..8]);
    record[80..84].copy_from_slice(&2u32.to_le_bytes());
    record[84..86].copy_from_slice(&120u16.to_le_bytes());
    assert!(matches_removal(&record, child, parent).unwrap());
    for action in [1u32, 3, 4, 5] {
        let mut other = record.clone();
        other[4..8].copy_from_slice(&action.to_le_bytes());
        assert!(!matches_removal(&other, child, parent).unwrap());
    }
    for length in [0u32, 1, u32::MAX] {
        let mut bad = record.clone();
        bad[80..84].copy_from_slice(&length.to_le_bytes());
        assert!(matches_removal(&bad, child, parent).is_err());
    }
    for next in [1u32, 8, 84, 88, u32::MAX] {
        let mut bad = record.clone();
        bad[..4].copy_from_slice(&next.to_le_bytes());
        assert!(matches_removal(&bad, child, parent).is_err());
    }
    let mut page = record.clone();
    page[..4].copy_from_slice(&88u32.to_le_bytes());
    page.extend_from_slice(&[0; 84]);
    assert!(
        matches_removal(&page, child, parent).is_err(),
        "matched earlier record cannot hide invalid later record"
    );
    child[15] = 1;
    assert!(matches_removal(&record, child, parent).is_err());
    assert!(matches_removal(&[], [1; 16], parent).is_err());
    println!("DG_NOTIFY_MALFORMED_PAGE_AND_ID_LOSS_REFUSED=1");
}

#[test]
fn postvalidation_hardlink_race_retains_current_child_and_original_capacity() {
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let label = parent.join("original");
    let path = label.join("owned");
    let alias = parent.join("outside_alias");
    std::fs::write(&path, b"original private data").unwrap();
    let mut capacity =
        super::git_private_capacity::GitPrivateCapacity::new(&label, 128 << 20, 0, &mut probe)
            .unwrap();
    let registered = std::fs::File::open(&path).unwrap();
    capacity.observe(&path, &registered, &mut probe).unwrap();
    drop(registered);
    let (file, name, _) = cursor
        .open_next_cleanup_child(&label, &capacity, &mut probe)
        .unwrap()
        .unwrap();
    assert_eq!(name, "owned");
    let source = path.clone();
    let target = alias.clone();
    super::windows_cleanup_mark_hook::WindowsCleanupMarkHook::install(move || {
        // 实际在最后核验之后建立另一链接，保留原File ID，不更换待删名称。
        std::fs::hard_link(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"original private data");
        println!("DG_HARDLINK_CREATED_AFTER_FINAL_VALIDATION=1");
    });
    let result = cursor.mark_cleanup_child(&file, &label, &capacity, &mut probe);
    assert!(
        result.is_err(),
        "a hardlink created after final validation must not qualify original deletion as complete"
    );
    assert!(
        cursor.cleanup_child_delete_requested(),
        "successful OS mutation must remain latched even when its seal fails"
    );
    assert!(cursor.next_entry(&mut probe).is_err());
    drop(file);
    assert_eq!(std::fs::read(&alias).unwrap(), b"original private data");
    assert!(
        cursor.confirm_cleanup_child_absent(&mut probe).is_err(),
        "member REMOVE must not release an unverified original object"
    );
    assert!(
        cursor
            .open_next_cleanup_child(&label, &capacity, &mut probe)
            .is_err()
    );
    println!("DG_HARDLINK_RACE_RETAINS_ORIGINAL_RESPONSIBILITY=1");
}

#[test]
fn empty_directory_post_mark_seal_and_original_removal_are_verified() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let label = parent.join("original");
    let path = label.join("owned_dir");
    std::fs::create_dir(&path).unwrap();
    let mut capacity =
        super::git_private_capacity::GitPrivateCapacity::new(&label, 128 << 20, 0, &mut probe)
            .unwrap();
    let registered = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_NO_RECALL | FILE_FLAG_OPEN_REPARSE_POINT,
        )
        .open(&path)
        .unwrap();
    capacity.observe(&path, &registered, &mut probe).unwrap();
    drop(registered);
    let (file, name, _) = cursor
        .open_next_cleanup_child(&label, &capacity, &mut probe)
        .unwrap()
        .unwrap();
    assert_eq!(name, "owned_dir");
    cursor
        .mark_cleanup_child(&file, &label, &capacity, &mut probe)
        .unwrap();
    assert!(cursor.cleanup_child_delete_requested());
    drop(file);
    assert!(cursor.confirm_cleanup_child_absent(&mut probe).unwrap());
    assert_fixture_absent(&path);
    assert!(
        cursor
            .open_next_cleanup_child(&label, &capacity, &mut probe)
            .unwrap()
            .is_none()
    );
    println!("DG_EMPTY_DIRECTORY_POST_MARK_SEAL_AND_REMOVE=1");
}

#[test]
fn vanished_unregistered_entry_advances_only_after_original_full_id_absence() {
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let root = parent.join("original");
    let capacity =
        super::git_private_capacity::GitPrivateCapacity::new(&root, 128 << 20, 0, &mut probe)
            .unwrap();
    let foreign = root.join("foreign");
    let moved = root.join("moved-foreign");
    std::fs::write(&foreign, b"original foreign").unwrap();
    assert!(
        cursor
            .open_next_cleanup_child(&root, &capacity, &mut probe)
            .is_err()
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), b"original foreign");
    std::fs::rename(&foreign, &moved).unwrap();
    std::fs::write(&foreign, b"replacement foreign").unwrap();
    assert!(
        cursor
            .open_next_cleanup_child(&root, &capacity, &mut probe)
            .is_err()
    );
    assert_eq!(std::fs::read(&moved).unwrap(), b"original foreign");
    assert_eq!(std::fs::read(&foreign).unwrap(), b"replacement foreign");
    std::fs::remove_file(&foreign).unwrap();
    // 即使原名称已经不存在，同卷仍存在的原 ID 也不能被当作消失。
    assert!(
        cursor
            .open_next_cleanup_child(&root, &capacity, &mut probe)
            .is_err()
    );
    assert_eq!(std::fs::read(&moved).unwrap(), b"original foreign");
    std::fs::remove_file(&moved).unwrap();
    println!("DG_WINDOWS_ENUMERATED_ABSENCE_RED_READY=1");
    let result = wait_for_foreign_completion(&mut cursor, &root, &capacity, &mut probe);
    assert!(
        matches!(result, Ok(None)),
        "full-ID absence must retire only vanished foreign entry: {result:?}"
    );
    println!("DG_WINDOWS_ENUMERATED_ABSENCE_GREEN=1");
}

#[test]
fn foreign_removal_preserves_hard_links_and_outstanding_external_handle() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let root = parent.join("original");
    let capacity =
        super::git_private_capacity::GitPrivateCapacity::new(&root, 128 << 20, 0, &mut probe)
            .unwrap();
    let foreign = root.join("foreign");
    let linked = root.join("linked-foreign");
    std::fs::write(&foreign, b"foreign payload").unwrap();
    assert!(
        cursor
            .open_next_cleanup_child(&root, &capacity, &mut probe)
            .is_err()
    );
    // 初次只读观察后创建另一硬链接；仅删除原名称不能证明原对象已最终移除。
    std::fs::hard_link(&foreign, &linked).unwrap();
    let external = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&foreign)
        .unwrap();
    std::fs::remove_file(&foreign).unwrap();
    assert!(
        cursor
            .open_next_cleanup_child(&root, &capacity, &mut probe)
            .is_err()
    );
    assert_eq!(std::fs::read(&linked).unwrap(), b"foreign payload");
    std::fs::remove_file(&linked).unwrap();
    assert!(
        cursor
            .open_next_cleanup_child(&root, &capacity, &mut probe)
            .is_err(),
        "external last-close responsibility must retain the original cursor"
    );
    drop(external);
    let result = wait_for_foreign_completion(&mut cursor, &root, &capacity, &mut probe);
    assert!(
        matches!(result, Ok(None)),
        "foreign last-close must complete original observation: {result:?}"
    );
    println!("DG_WINDOWS_FOREIGN_LINKS_AND_LAST_CLOSE=1");
}

// 同一原预算内消费异步通知；不重建 owner、放大期限或把失败投影为成功。
fn wait_for_foreign_completion(
    cursor: &mut WindowsGitDirectoryCursor,
    root: &std::path::Path,
    capacity: &super::git_private_capacity::GitPrivateCapacity,
    probe: &mut ProbeBudget,
) -> std::io::Result<Option<(std::fs::File, OsString, u32)>> {
    loop {
        let result = cursor.open_next_cleanup_child(root, capacity, probe);
        if result.is_ok() || probe.check().is_err() {
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn modified_original_private_artifact_is_rejected_as_evidence_but_actually_disposed() {
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let label = parent.join("original");
    let path = label.join("owned");
    let mut capacity =
        super::git_private_capacity::GitPrivateCapacity::new(&label, 128 << 20, 0, &mut probe)
            .unwrap();
    std::fs::write(&path, b"original").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let identity = GitPrivateAllocation::from_file(&file).unwrap();
    let original_modified = file.metadata().unwrap().modified().unwrap();
    capacity.observe(&path, &file, &mut probe).unwrap();
    drop(file);
    std::fs::write(&path, b"modified").unwrap();
    // 同长度连续写入可能落在同一原生时钟刻度；显式改变原句柄时间，确保版本拒绝前提真实成立。
    let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    writer
        .set_times(
            std::fs::FileTimes::new().set_modified(
                original_modified
                    .checked_add(std::time::Duration::from_secs(60))
                    .unwrap(),
            ),
        )
        .unwrap();
    drop(writer);
    let modified = std::fs::File::open(&path).unwrap();
    let modified_identity = GitPrivateAllocation::from_file(&modified).unwrap();
    assert!(identity.same_identity(&modified_identity));
    assert!(!identity.same_version(&modified_identity));
    assert_eq!(std::fs::read(&path).unwrap(), b"modified");
    assert!(
        capacity
            .check_identity(&path, &modified, false)
            .unwrap_err()
            .contains("version changed")
    );
    drop(modified);
    println!("DG_MODIFIED_PRIVATE_IDENTITY_REJECTION_RED_READY=1");
    let result = cursor.open_next_cleanup_child(&label, &capacity, &mut probe);
    assert!(
        result.is_ok(),
        "modified original private artifact must remain disposable: {result:?}"
    );
    let (file, name, _) = result.unwrap().unwrap();
    assert_eq!(name, OsString::from("owned"));
    assert!(identity.same_identity(&GitPrivateAllocation::from_file(&file).unwrap()));
    assert!(
        capacity
            .check_identity(&path, &file, false)
            .unwrap_err()
            .contains("version changed")
    );
    cursor
        .mark_cleanup_child(&file, &label, &capacity, &mut probe)
        .unwrap();
    drop(file);
    loop {
        let result = cursor.confirm_cleanup_child_absent(&mut probe);
        if matches!(result, Ok(true)) {
            break;
        }
        probe.check().unwrap_or_else(|deadline| {
            panic!("original mutation cleanup budget exhausted: {deadline}; {result:?}")
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_fixture_absent(&path);
    assert!(
        capacity.registered(&path),
        "cleanup must not replace trusted ledger evidence"
    );
    assert!(
        cursor
            .open_next_cleanup_child(&label, &capacity, &mut probe)
            .unwrap()
            .is_none()
    );
    println!("DG_MODIFIED_PRIVATE_DISPOSAL_WITHOUT_EVIDENCE_TRUST=1");
}

#[test]
fn cleanup_child_delete_lease_freezes_original_parent_until_actual_removal() {
    let (_temp, parent, _root, mut cursor, mut probe) = cleanup_fixture();
    let label = parent.join("original");
    let path = label.join("owned");
    let other_parent = parent.join("other-parent");
    std::fs::create_dir(&other_parent).unwrap();
    let moved = other_parent.join("moved");
    let mut capacity =
        super::git_private_capacity::GitPrivateCapacity::new(&label, 128 << 20, 0, &mut probe)
            .unwrap();
    std::fs::write(&path, b"original payload").unwrap();
    let original = std::fs::File::open(&path).unwrap();
    let identity = GitPrivateAllocation::from_file(&original).unwrap();
    capacity.observe(&path, &original, &mut probe).unwrap();
    drop(original);
    let (file, name, _) = cursor
        .open_next_cleanup_child(&label, &capacity, &mut probe)
        .unwrap()
        .unwrap();
    assert_eq!(name, OsString::from("owned"));
    let moved_result = std::fs::rename(&path, &moved);
    assert!(identity.same_identity(&GitPrivateAllocation::from_file(&file).unwrap()));
    println!("DG_ORIGINAL_CHILD_ASSOCIATION_RED_READY=1");
    assert_eq!(
        moved_result
            .as_ref()
            .err()
            .and_then(std::io::Error::raw_os_error),
        Some(32),
        "original child DELETE lease must freeze parent association: {moved_result:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original payload");
    assert!(!moved.exists());
    cursor
        .mark_cleanup_child(&file, &label, &capacity, &mut probe)
        .unwrap();
    drop(file);
    loop {
        let result = cursor.confirm_cleanup_child_absent(&mut probe);
        if matches!(result, Ok(true)) {
            break;
        }
        probe.check().unwrap_or_else(|deadline| {
            panic!("original child removal budget exhausted: {deadline}; {result:?}")
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_fixture_absent(&path);
    assert!(!moved.exists());
    assert!(
        cursor
            .open_next_cleanup_child(&label, &capacity, &mut probe)
            .unwrap()
            .is_none()
    );
    println!("DG_ORIGINAL_CHILD_DELETE_LEASE_AND_ACTUAL_REMOVAL=1");
}
