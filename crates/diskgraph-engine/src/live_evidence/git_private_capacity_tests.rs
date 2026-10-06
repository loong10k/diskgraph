//! 私有 Git 视图的真实分配与卷余量门禁；所有写入只发生在独占临时目录。

use super::ProbeLimits;
use super::git_private_allocation::GitPrivateAllocation;
use super::git_private_capacity::GitPrivateCapacity;
use super::git_private_directory::GitPrivateDirectory;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;
use std::path::Path;

fn probe() -> ProbeBudget {
    ProbeBudget::new(&ProbeLimits::default()).unwrap()
}

#[cfg(unix)]
fn native_allocation(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).unwrap().blocks() * 512
}

#[cfg(windows)]
fn native_allocation(path: &Path) -> u64 {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_STANDARD_INFO, FileStandardInfo,
        GetFileInformationByHandleEx,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    let mut standard = FILE_STANDARD_INFO::default();
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStandardInfo,
                (&mut standard as *mut FILE_STANDARD_INFO).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        },
        0
    );
    u64::try_from(standard.AllocationSize).unwrap()
}

fn content() -> Vec<u8> {
    let mut state = 19u32;
    (0..8193)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

#[test]
fn many_short_files_refuse_reported_allocation_before_logical_bytes_exhaust_the_quota() {
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(48 * 1024, 0, &mut budget).unwrap();
    let root = directory.path().to_owned();
    let bytes = content();
    let first = root.join("first");
    directory.write(&first, &bytes, &mut budget).unwrap();
    let allocated = native_allocation(&first);
    assert!(
        allocated > bytes.len() as u64,
        "native fixture must distinguish allocation from length"
    );
    let mut accepted = 1;
    let mut refusal = None;
    for number in 0..8 {
        match directory.write(&root.join(format!("short-{number}")), &bytes, &mut budget) {
            Ok(()) => accepted += 1,
            Err(error) => {
                refusal = Some(error);
                break;
            }
        }
    }
    let error = refusal.expect("allocated short files exceeded quota without refusal");
    assert!(error.contains("allocation"), "{error}");
    assert!(
        (accepted + 1) * bytes.len() < 48 * 1024,
        "refusal must arise before the next logical bytes exhaust quota"
    );
    directory.complete::<()>(Err(error)).unwrap_err();
    // 原资源拒绝保持不变；Windows 的目录清理可能显式移交原恢复池。
    // 先结束原目录及会话，再由同一个外部 owner 实际排空，不能将 complete 错误当作已删除。
    drop(directory);
    drop(budget);
    assert!(!root.exists());
}

#[test]
fn real_volume_capacity_cannot_meet_larger_required_headroom() {
    let root = std::env::temp_dir().canonicalize().unwrap();
    let space = diskgraph_disktree_core::space::space_info(&root).unwrap();
    assert!(space.available <= space.total);
    // 并行清理会增加 available；真实卷总量 + 1 是稳定不可满足的余量，不模拟原生查询。
    let error = GitPrivateDirectory::with_limits(
        128 << 20,
        space.total.checked_add(1).unwrap(),
        &mut probe(),
    )
    .err()
    .expect("native available-space query was not enforced");
    assert!(error.contains("space"), "{error}");
}

#[test]
fn unknown_native_volume_query_is_not_reported_as_unlimited_space() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    assert!(GitPrivateCapacity::check_volume(&missing, 0, 0, &mut probe()).is_err());
}

#[test]
fn owned_overwrites_replace_the_existing_allocation_charge() {
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(32 * 1024, 0, &mut budget).unwrap();
    let root = directory.path().to_owned();
    let path = root.join("owned");
    for _ in 0..8 {
        directory.write(&path, &content(), &mut budget).unwrap();
    }
    directory.verify_capacity(&mut budget).unwrap();
    directory.complete(Ok(())).unwrap();
    assert!(!root.exists());
}

#[test]
fn unknown_files_and_outside_paths_are_never_overwritten() {
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(1 << 20, 0, &mut budget).unwrap();
    let unknown = directory.path().join("unknown");
    std::fs::write(&unknown, b"original").unwrap();
    assert!(
        directory
            .write(&unknown, b"replacement", &mut budget)
            .is_err()
    );
    assert_eq!(std::fs::read(&unknown).unwrap(), b"original");
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    assert!(
        directory
            .write(&outside, b"replacement", &mut budget)
            .is_err()
    );
    assert!(!outside.exists());
    assert!(
        directory
            .create_dir_all(&directory.path().join("../escape"), &mut budget)
            .is_err()
    );
    std::fs::remove_file(unknown).unwrap();
    directory.complete(Ok(())).unwrap();
}

#[test]
fn final_verification_rejects_unregistered_data_and_changed_allocation() {
    let mut second_budget = probe();
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(48 * 1024, 0, &mut budget).unwrap();
    let path = directory.path().join("owned");
    directory.write(&path, b"small", &mut budget).unwrap();
    std::fs::write(&path, vec![19; 64 * 1024]).unwrap();
    assert!(directory.verify_capacity(&mut budget).is_err());
    let mut second = GitPrivateDirectory::with_limits(1 << 20, 0, &mut second_budget).unwrap();
    let foreign = second.path().join("unregistered");
    std::fs::write(&foreign, b"foreign").unwrap();
    assert!(second.verify_capacity(&mut second_budget).is_err());
    // 先验证拒绝外来数据，再只删除本测试创建的外来文件，让原 owner 清理登记对象。
    std::fs::remove_file(foreign).unwrap();
    second.complete(Ok(())).unwrap();
    // 原生版本变化不能伪造恢复；负断言完成后由制造变化的测试移除该文件，原 owner 清理根。
    std::fs::remove_file(&path).unwrap();
    directory.complete(Ok(())).unwrap();
}

#[test]
fn failed_write_keeps_its_diagnostic_and_private_owner_is_cleaned() {
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(1 << 20, 0, &mut budget).unwrap();
    let root = directory.path().to_owned();
    let nested = root.join("nested");
    directory.create_dir_all(&nested, &mut budget).unwrap();
    let retained = directory.path().join("original-nested");
    std::fs::rename(&nested, &retained).unwrap();
    let error = directory
        .write(&nested.join("data"), b"data", &mut budget)
        .unwrap_err();
    assert!(
        error.contains("private") || error.contains("Git"),
        "{error}"
    );
    // 原身份恢复后仍以原错误显式完成；不把账本之外删除登记目录当成恢复成功。
    std::fs::rename(&retained, &nested).unwrap();
    let preserved = directory.complete::<()>(Err(error.clone())).unwrap_err();
    assert!(preserved.contains(&error));
    assert!(!root.exists());
}

#[test]
fn native_allocation_observation_is_not_apparent_file_length() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("short");
    let bytes = content();
    std::fs::write(&path, &bytes).unwrap();
    let actual = native_allocation(&path);
    assert!(actual > bytes.len() as u64);
    let observed = GitPrivateAllocation::capture(&path).unwrap();
    assert_eq!(observed.bytes(), actual);
}

#[test]
fn modified_time_is_set_through_the_registered_handle_and_unknown_files_stay_unchanged() {
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(1 << 20, 0, &mut budget).unwrap();
    let path = directory.path().join("index");
    directory
        .write(&path, b"sample index", &mut budget)
        .unwrap();
    let time = std::time::UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123_456_700);
    directory.set_modified(&path, time, &mut budget).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), time);
    let unknown = directory.path().join("unregistered");
    std::fs::write(&unknown, b"foreign").unwrap();
    let original = std::fs::metadata(&unknown).unwrap().modified().unwrap();
    assert!(directory.set_modified(&unknown, time, &mut budget).is_err());
    assert_eq!(
        std::fs::metadata(&unknown).unwrap().modified().unwrap(),
        original
    );
    std::fs::remove_file(unknown).unwrap();
    directory.complete(Ok(())).unwrap();
}

#[test]
fn changed_registered_file_identity_cannot_be_truncated() {
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(1 << 20, 0, &mut budget).unwrap();
    let path = directory.path().join("owned");
    directory.write(&path, b"owned", &mut budget).unwrap();
    std::fs::rename(&path, directory.path().join("previous")).unwrap();
    std::fs::write(&path, b"foreign replacement").unwrap();
    assert!(directory.write(&path, b"replacement", &mut budget).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign replacement");
    std::fs::remove_file(&path).unwrap();
    // 文件 rename 已改变原生版本，测试移除自己改动的两份文件，不伪造版本恢复。
    std::fs::remove_file(directory.path().join("previous")).unwrap();
    directory.complete(Ok(())).unwrap();
}

#[cfg(unix)]
#[test]
fn substituted_parent_symlink_does_not_write_outside_the_owner() {
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(1 << 20, 0, &mut budget).unwrap();
    let nested = directory.path().join("nested");
    directory.create_dir_all(&nested, &mut budget).unwrap();
    std::fs::remove_dir(&nested).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), &nested).unwrap();
    assert!(
        directory
            .write(&nested.join("foreign"), b"data", &mut budget)
            .is_err()
    );
    assert!(!outside.path().join("foreign").exists());
}

#[test]
fn zero_allocation_files_still_reach_the_real_owner_entry_limit() {
    let limits = ProbeLimits {
        timeout: std::time::Duration::from_secs(120),
        ..ProbeLimits::default()
    };
    let mut budget = ProbeBudget::new(&limits).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut budget).unwrap();
    let root = directory.path().to_owned();
    let first = root.join("empty-0");
    directory.write(&first, b"", &mut budget).unwrap();
    assert_eq!(
        native_allocation(&first),
        0,
        "fixture must really report no allocated file data"
    );
    for number in 1..32_767 {
        directory
            .write(&root.join(format!("empty-{number}")), b"", &mut budget)
            .unwrap();
    }
    let error = directory
        .write(&root.join("one-too-many"), b"", &mut budget)
        .unwrap_err();
    assert!(error.contains("entry limit"), "{error}");
    directory.complete::<()>(Err(error)).unwrap_err();
    // 原资源拒绝保持不变；Windows 的目录清理可能显式移交原恢复池。
    // 先结束原目录及会话，再由同一个外部 owner 实际排空，不能将 complete 错误当作已删除。
    drop(directory);
    drop(budget);
    assert!(!root.exists());
}

#[test]
fn existing_directory_creation_does_not_adopt_a_replacement_identity() {
    let mut budget = probe();
    let mut directory = GitPrivateDirectory::with_limits(1 << 20, 0, &mut budget).unwrap();
    let nested = directory.path().join("nested");
    directory.create_dir_all(&nested, &mut budget).unwrap();
    std::fs::rename(&nested, directory.path().join("original")).unwrap();
    std::fs::create_dir(&nested).unwrap();
    assert!(directory.create_dir_all(&nested, &mut budget).is_err());
    assert!(
        directory
            .write(&nested.join("data"), b"data", &mut budget)
            .is_err()
    );
    assert!(!nested.join("data").exists());
    std::fs::remove_dir(&nested).unwrap();
    std::fs::rename(directory.path().join("original"), &nested).unwrap();
    directory.complete(Ok(())).unwrap();
}
