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
    let mut record = FILE_ID_EXTD_DIR_INFO::default();
    record.FileNameLength = 2;
    record.FileName = [120];
    record.FileId.Identifier = [1; 16];
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
