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
