//! 原删除通知跨清理轮次的真实 Windows 预算回归；不以路径缺失授予完成。
use super::ProbeLimits;
use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use super::windows_git_removal_observation::WindowsGitRemovalObservation;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::time::Duration;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

#[test]
fn original_removal_notification_survives_a_cleanup_round_output_limit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original");
    std::fs::write(&path, b"original identity").unwrap();
    let parent = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_NO_RECALL | FILE_FLAG_OPEN_REPARSE_POINT,
        )
        .open(directory.path())
        .unwrap();
    let child = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_NO_RECALL | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .unwrap();
    let identity = GitPrivateAllocation::from_file(&child).unwrap();
    let mut exhausted = ProbeBudget::new(&ProbeLimits {
        max_output_bytes: 0,
        ..ProbeLimits::default()
    })
    .unwrap();
    let mut owner = None;
    WindowsGitRemovalObservation::prepare_into(&parent, &identity, &mut exhausted, &mut owner)
        .unwrap();
    // 仅测试 actor 删除自己的文件；产品原订阅与原完整身份始终不更换。
    std::fs::remove_file(&path).unwrap();
    drop(child);
    loop {
        match owner.as_mut().unwrap().confirm(&identity, &mut exhausted) {
            Ok(false) => {
                exhausted.check().unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(true) => panic!("zero-byte round confirmed an uncharged removal record"),
            Err(error) => {
                assert!(
                    error.to_string().contains("cumulative output byte limit"),
                    "{error}"
                );
                break;
            }
        }
    }
    // 新清理轮不是业务请求续期；沿同一原 owner 重试，不能改用路径缺名或重新订阅。
    let mut next_round = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    while !owner
        .as_mut()
        .unwrap()
        .confirm(&identity, &mut next_round)
        .unwrap()
    {
        next_round.check().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    println!("DG_ORIGINAL_REMOVAL_SURVIVES_ROUND_OUTPUT_LIMIT=1");
}

#[test]
fn failed_stream_preserves_created_file_ownership_for_original_cleanup() {
    use super::git_private_directory::GitPrivateDirectory;
    use super::native_probe_test_budget::NativeProbeTestBudget;
    use std::io::Write;

    let mut budget = NativeProbeTestBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut budget).unwrap();
    let root = directory.path().to_owned();
    let path = root.join("unfinished-stream");
    let result: Result<(), String> = directory.write_stream(&path, 32, &mut budget, |file, _| {
        file.write_all(b"partial").unwrap();
        Err("original stream failure".into())
    });
    assert_eq!(result.unwrap_err(), "original stream failure");
    let cleanup = directory.complete::<()>(Ok(()));
    // RED 阶段也实际恢复原 owner：测试 actor 仅收回本夹具的已知文件，不替代产品验收。
    if cleanup.is_err() {
        std::fs::remove_file(&path).unwrap();
    }
    drop(directory);
    drop(budget);
    assert!(
        !root.exists(),
        "original recovery must finish before the assertion"
    );
    assert!(
        cleanup.is_ok(),
        "created stream became an unregistered foreign entry: {cleanup:?}"
    );
}

#[test]
fn cancelled_stream_preserves_original_cleanup_and_cancellation() {
    use super::git_private_directory::GitPrivateDirectory;
    use super::native_probe_test_budget::NativeProbeTestBudget;
    use std::io::Write;
    use std::sync::atomic::Ordering;

    let limits = ProbeLimits::default();
    let mut budget = NativeProbeTestBudget::new(&limits).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut budget).unwrap();
    let root = directory.path().to_owned();
    let path = root.join("cancelled-stream");
    let result = directory.write_stream(&path, 32, &mut budget, |file, probe| {
        file.write_all(b"partial").unwrap();
        limits.cancel.store(true, Ordering::Release);
        probe.check().map_err(|error| error.to_string())
    });
    let original = result.unwrap_err();
    assert!(original.contains("cancel"), "{original}");
    let completion = directory.complete::<()>(Err(original.clone()));
    drop(directory);
    drop(budget);
    assert!(!root.exists());
    assert!(completion.unwrap_err().starts_with(&original));
}

#[test]
fn panicking_stream_keeps_original_created_object_recoverable() {
    use super::git_private_directory::GitPrivateDirectory;
    use super::native_probe_test_budget::NativeProbeTestBudget;
    use std::io::Write;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let mut budget = NativeProbeTestBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut budget).unwrap();
    let root = directory.path().to_owned();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _: Result<(), String> = directory.write_stream(
            &root.join("panicking-stream"),
            32,
            &mut budget,
            |file, _| {
                file.write_all(b"partial").unwrap();
                panic!("original stream panic")
            },
        );
    }));
    assert!(result.is_err());
    let completion = directory.complete::<()>(Ok(()));
    drop(directory);
    drop(budget);
    assert!(!root.exists());
    assert!(completion.is_ok(), "{completion:?}");
}

#[test]
#[ignore = "真实 Windows 私有文件创建与原 owner 清理成本诊断"]
fn native_private_creation_and_original_cleanup_cost() {
    use super::git_private_directory::GitPrivateDirectory;
    use super::native_probe_test_budget::NativeProbeTestBudget;
    use std::time::Instant;

    for round in 0..3 {
        // 保留产品原 15s 采样期限、真实 200 个零分配对象和原恢复责任。
        let mut budget = NativeProbeTestBudget::new(&ProbeLimits::default()).unwrap();
        let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut budget).unwrap();
        let root = directory.path().to_owned();
        let started = Instant::now();
        for index in 0..200 {
            directory
                .write(&root.join(format!("empty-{index}")), b"", &mut budget)
                .unwrap();
        }
        let creation = started.elapsed();
        let cleanup = Instant::now();
        let original_completion = directory.complete::<()>(Ok(()));
        drop(directory);
        drop(budget);
        assert!(
            !root.exists(),
            "original recovery must actually delete its own root"
        );
        println!(
            "DG_PRIVATE_DIRECTORY_COST round={round} entries=200 creation_us={} cleanup_us={} original_completion={original_completion:?}",
            creation.as_micros(),
            cleanup.elapsed().as_micros()
        );
    }
}
