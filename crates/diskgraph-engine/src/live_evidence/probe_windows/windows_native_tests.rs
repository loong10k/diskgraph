use super::windows_probe_child::WindowsProbeChild;
use crate::live_evidence::ProbeLimits;
use crate::live_evidence::probe_budget::ProbeBudget;
use crate::live_evidence::probe_execution::run_probe;
use crate::live_evidence::probe_failure::ProbeFailure;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
    QueryInformationJobObject,
};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

use super::owned_handle::OwnedHandle;

fn fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "live_evidence::probe_windows::windows_native_tests::windows_native_fixture",
            "--nocapture",
        ])
        .env_clear()
        .env("DG_WINDOWS_NATIVE_CASE", mode);
    command
}

#[test]
fn windows_native_fixture() {
    let Ok(mode) = std::env::var("DG_WINDOWS_NATIVE_CASE") else {
        return;
    };
    match mode.as_str() {
        "unlisted_handle" => {
            let value: usize = std::env::var("DG_WINDOWS_EVENT_HANDLE")
                .unwrap()
                .parse()
                .unwrap();
            let result = unsafe { SetEvent(value as HANDLE) };
            std::io::stdout()
                .write_all(if result == 0 {
                    b"isolated".as_slice()
                } else {
                    b"leaked".as_slice()
                })
                .unwrap();
        }
        "stamp" => {
            let marker = std::env::var_os("DG_WINDOWS_MARKER").unwrap();
            std::fs::write(marker, b"started").unwrap();
        }
        "zero_then_payload" => {
            for (stream, payload) in [
                (STD_OUTPUT_HANDLE, b"stdout-after-zero".as_slice()),
                (STD_ERROR_HANDLE, b"stderr-after-zero".as_slice()),
            ] {
                let handle = unsafe { GetStdHandle(stream) };
                let mut written = u32::MAX;
                let unused = 0u8;
                assert_ne!(
                    unsafe {
                        WriteFile(
                            handle,
                            &raw const unused,
                            0,
                            &mut written,
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
                assert_eq!(written, 0);
                assert_ne!(
                    unsafe {
                        WriteFile(
                            handle,
                            payload.as_ptr(),
                            payload.len() as u32,
                            &mut written,
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
                assert_eq!(written as usize, payload.len());
            }
        }
        "silent" => std::thread::sleep(std::time::Duration::from_secs(2)),
        "ordinary_descendant" => {
            let marker = std::env::var_os("DG_WINDOWS_MARKER").unwrap();
            let mut descendant = fixture("heartbeat");
            descendant
                .env("DG_WINDOWS_MARKER", &marker)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // 验收特意让 leader 先退出，外层 Job 负责回收普通后代；fixture 自身最多存活 10 秒。
            #[allow(clippy::zombie_processes)]
            let _child = descendant.spawn().unwrap();
            let start = Instant::now();
            while !std::path::Path::new(&marker).exists() {
                assert!(start.elapsed() < Duration::from_secs(3));
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        "heartbeat" => {
            let marker = std::env::var_os("DG_WINDOWS_MARKER").unwrap();
            std::fs::write(marker, b"started").unwrap();
            std::thread::sleep(Duration::from_secs(10));
        }
        _ => panic!("unknown native fixture mode"),
    }
}

#[test]
fn handle_list_excludes_other_inheritable_parent_handles() {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let event = OwnedHandle::from_raw(
        unsafe { CreateEventW(&attributes, 1, 0, std::ptr::null()) },
        "CreateEventW(test unlisted inheritable handle)",
    )
    .unwrap();
    let mut command = fixture("unlisted_handle");
    command.env(
        "DG_WINDOWS_EVENT_HANDLE",
        (event.as_raw() as usize).to_string(),
    );
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let output = run_probe(&mut command, &mut budget).unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(
        output
            .stdout
            .windows(b"isolated".len())
            .any(|slice| slice == b"isolated")
    );
    assert_eq!(
        unsafe { WaitForSingleObject(event.as_raw(), 0) },
        WAIT_TIMEOUT
    );
}

#[test]
fn cleanup_confirms_both_pending_reads_before_buffers_drop() {
    let mut command = fixture("silent");
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut child = WindowsProbeChild::spawn(&mut command, &mut budget).unwrap();
    while child.read_stdout().unwrap().is_some() {}
    while child.read_stderr().unwrap().is_some() {}
    assert!(!child.stdout_eof());
    assert!(!child.stderr_eof());
    child.cleanup().unwrap();
}

#[test]
fn post_create_failure_cleans_suspended_job_before_returning() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("started");
    let mut command = fixture("stamp");
    command
        .env("DG_WINDOWS_MARKER", &marker)
        .env("DG_WINDOWS_NATIVE_FAULT", "post_create");
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let error = match WindowsProbeChild::spawn(&mut command, &mut budget) {
        Ok(_) => panic!("post-create injection unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        ProbeFailure::Unsupported("injected post-create failure")
    ));
    assert!(!marker.exists());
}

#[test]
fn zero_length_writes_on_both_pipes_do_not_hide_following_payload() {
    let mut command = fixture("zero_then_payload");
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let output = run_probe(&mut command, &mut budget).unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(
        output
            .stdout
            .windows(b"stdout-after-zero".len())
            .any(|bytes| bytes == b"stdout-after-zero")
    );
    assert!(
        output
            .stderr
            .windows(b"stderr-after-zero".len())
            .any(|bytes| bytes == b"stderr-after-zero")
    );
}

fn job_accounting(job: HANDLE) -> JOBOBJECT_BASIC_ACCOUNTING_INFORMATION {
    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    assert_ne!(
        unsafe {
            QueryInformationJobObject(
                job,
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                std::mem::size_of_val(&accounting) as u32,
                std::ptr::null_mut(),
            )
        },
        0
    );
    accounting
}

#[test]
fn cleanup_waits_until_job_has_no_active_descendant_after_leader_exit() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("descendant-started");
    let mut command = fixture("ordinary_descendant");
    command.env("DG_WINDOWS_MARKER", &marker);
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut child = WindowsProbeChild::spawn(&mut command, &mut budget).unwrap();
    let job = child.duplicate_job_for_test().unwrap();
    let start = Instant::now();
    while !child.poll().unwrap() {
        assert!(start.elapsed() < Duration::from_secs(4));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(marker.exists());
    let before = job_accounting(job.as_raw());
    assert!(
        before.TotalProcesses >= 2,
        "ordinary descendant did not join the Job"
    );
    assert!(
        before.ActiveProcesses >= 1,
        "descendant ended before cleanup"
    );
    child.cleanup().unwrap();
    assert_eq!(job_accounting(job.as_raw()).ActiveProcesses, 0);
}

#[test]
fn externally_held_exited_process_handle_bounds_accounting_wait() {
    let mut command = fixture("silent");
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut child = WindowsProbeChild::spawn(&mut command, &mut budget).unwrap();
    let external_process_reference = child.duplicate_leader_for_test().unwrap();
    let started = Instant::now();
    let error = child.cleanup().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("ActiveProcesses remained nonzero"),
        "{error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "cleanup exceeded its accounting observation window"
    );
    drop(external_process_reference);
}
