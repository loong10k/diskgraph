//! Windows 原句柄属性重复捕获的成本诊断；不修改原请求预算。
use super::git_private_allocation::GitPrivateAllocation;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::time::Instant;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
};

#[test]
#[ignore = "原生阶段成本诊断，显式运行；不能替代完整并发性能验收"]
fn native_allocation_repeated_capture_cost() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original");
    std::fs::write(&path, b"original file version").unwrap();
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_NO_RECALL | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .unwrap();
    let original = GitPrivateAllocation::from_file(&file).unwrap();
    for round in 0..3 {
        let started = Instant::now();
        for _ in 0..4096 {
            let current = GitPrivateAllocation::from_file(&file).unwrap();
            assert!(original.same_version(&current));
            assert_eq!(original.bytes(), current.bytes());
        }
        println!(
            "DG_NATIVE_ALLOCATION_CAPTURE round={round} captures=4096 elapsed_us={}",
            started.elapsed().as_micros()
        );
    }
}
