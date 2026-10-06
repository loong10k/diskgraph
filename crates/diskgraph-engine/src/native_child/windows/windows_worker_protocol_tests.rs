//! 当前真实worker的Windows协议及退出门禁；不授予产品镜像执行许可。
use super::{OwnedHandle, WindowsChild};
use crate::native_child::ChildInputMode;
use crate::scan_worker_driver::ScanWorkerDriver;
use crate::scan_worker_failure::ScanWorkerFailure;
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{ExecutionOutcome, ProtocolLimits, WorkerRequest};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn check(deadline: Instant) -> std::io::Result<()> {
    if Instant::now() >= deadline {
        Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
    } else {
        Ok(())
    }
}

fn start(root: &Path) -> (ScanWorkerDriver, OwnedHandle, Instant) {
    let image = PathBuf::from(
        std::env::var_os("DISKGRAPH_WINDOWS_WORKER_QUALIFICATION")
            .expect("qualification requires the actual current Cargo-built worker"),
    );
    assert!(image.is_absolute() && image.is_file() && !image.is_symlink());
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut command = Command::new(image);
    command.env_clear().current_dir(root);
    let mut owner = None;
    WindowsChild::spawn_into_with_admission(
        &mut command,
        ChildInputMode::WorkerControl,
        &mut owner,
        || check(deadline),
        || check(deadline),
    )
    .unwrap();
    let job = owner.as_ref().unwrap().duplicate_job_for_test().unwrap();
    let options = ScanOptions {
        apparent_size: true,
        follow_links: false,
        include_hidden: true,
        one_filesystem: true,
        max_depth: Some(8),
        dedup_hardlinks: true,
        metric: diskgraph_disktree_core::tree::Metric::Bytes,
    };
    let request = WorkerRequest::scan(
        root,
        &options,
        ProtocolLimits {
            max_frame_bytes: 65536,
            max_stream_bytes: 1048576,
            max_nodes: 4096,
            max_depth: 64,
        },
    )
    .unwrap();
    // 配置失败时原owner仍在外槽；成功后才移交真实driver。
    let driver = ScanWorkerDriver::new(
        &mut owner,
        request,
        (
            env!("DISKGRAPH_ENGINE_TARGET"),
            "158f9cc2f0b332194a3ffc5acec47760c99146d8",
        ),
        deadline,
        65536,
    )
    .unwrap();
    (driver, job, deadline)
}

fn assert_job_empty(job: &OwnedHandle) {
    use windows_sys::Win32::System::JobObjects::{
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
        QueryInformationJobObject,
    };
    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    assert_ne!(
        unsafe {
            QueryInformationJobObject(
                job.as_raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                std::mem::size_of_val(&accounting) as u32,
                std::ptr::null_mut(),
            )
        },
        0
    );
    assert_eq!(
        accounting.ActiveProcesses, 0,
        "original Job still has a live worker"
    );
}

#[test]
fn current_worker_scans_raw_utf16_and_exits_normally() {
    use std::os::windows::ffi::OsStringExt;
    let directory = tempfile::tempdir().unwrap();
    let root = directory
        .path()
        .join(std::ffi::OsString::from_wide(&[0xd800, 0x61]));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("first"), [1_u8; 17]).unwrap();
    std::fs::write(root.join("plain"), [2_u8; 23]).unwrap();
    let (mut driver, job, deadline) = start(&root);
    let outcome = loop {
        if let Some(outcome) = driver.poll(false, || check(deadline)).unwrap() {
            break outcome;
        }
        std::thread::sleep(driver.next_poll_delay().min(Duration::from_millis(1)));
    };
    let ExecutionOutcome::Tree(tree) = outcome else {
        panic!("real scan must return its complete tree")
    };
    assert_eq!(tree.files, 2);
    assert_eq!(tree.bytes, 40);
    assert_eq!(tree.children.len(), 2);
    assert!(
        tree.children
            .iter()
            .any(|node| node.name.as_ref() == "first")
    );
    assert_eq!(driver.exit_code(), Some(0));
    assert_job_empty(&job);
    eprintln!("DG_CURRENT_WINDOWS_WORKER_RAW_SCAN_NORMAL_EXIT=1");
}

#[test]
fn current_worker_original_cancel_after_birth_is_reaped() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("plain"), [2_u8; 23]).unwrap();
    let (mut driver, job, _) = start(directory.path());
    let failure = driver
        .poll(false, || {
            Err::<(), _>(std::io::Error::from(std::io::ErrorKind::Interrupted))
        })
        .unwrap_err();
    assert!(
        matches!(failure, ScanWorkerFailure::Checkpoint { primary, cleanup: None }
        if primary.kind() == std::io::ErrorKind::Interrupted)
    );
    assert_job_empty(&job);
    eprintln!("DG_CURRENT_WINDOWS_WORKER_ORIGINAL_CANCEL_REAPED=1");
}
