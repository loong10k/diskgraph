//! 固定 Linux helper 的线程派生约束验收；不把它声明为文件来源沙箱或通用进程组视图。

use super::linux_atomic_birth_fixture::AtomicBirthFixture;
use super::linux_scanner_sandbox_test_support as fixture;
use super::{ChildError, ChildSpawnError, ControlWriteStatus, UnixChild};
use std::ffi::OsStr;
use std::io;
use std::process::Command;

#[test]
fn kernel_filter_keeps_real_pthreads_and_rejects_process_and_namespace_derivation() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("report", directory.path());
    let pid = fixture::identity(directory.path());
    child.request_control_close().unwrap();
    let output = fixture::drain(&mut child);
    let report: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(report["pthread_create"], 0);
    assert_eq!(report["pthread_join"], 0);
    assert_ne!(report["thread_tid"].as_i64().unwrap(), i64::from(pid));
    assert_eq!(report["thread_tgid"], pid);
    for field in [
        "thread_fork_errno",
        "fork_errno",
        "vfork_errno",
        "process_clone_errno",
        "upper_flags_errno",
        "no_vm_errno",
        "no_sighand_errno",
        "namespace_flags_errno",
        "unshare_errno",
        "setns_errno",
        "setsid_errno",
        "setpgid_errno",
        "ptrace_errno",
        "uring_setup_errno",
        "uring_enter_errno",
        "uring_register_errno",
    ] {
        assert_eq!(report[field], libc::EPERM, "actual {field}: {report}");
    }
    assert_eq!(report["clone3_errno"], libc::ENOSYS);
    assert!(fixture::finish(&mut child) >= 2);
    assert_eq!(child.exit_code(), Some(0));
    fixture::reaped(pid);
}

#[test]
fn leader_pthread_exit_does_not_permit_reaping_its_two_live_kernel_threads() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("threads", directory.path());
    let pid = fixture::identity(directory.path());
    let pidfd = fixture::pidfd(pid);
    fixture::wait_file(directory.path(), "main_leaving");
    fixture::main_zombie_with_threads(pid);
    for index in 0..2 {
        assert_ne!(
            fixture::scalar(directory.path(), &format!("tid_{index}")),
            i64::from(pid)
        );
        fixture::advancing(directory.path(), index);
    }
    assert!(!fixture::pidfd_ready(&pidfd));
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    assert!(fixture::drain(&mut child).is_empty());
    assert!(!child.poll().unwrap());
    let mut checks = 0;
    assert!(
        !child
            .poll_normal_exit(|| {
                checks += 1;
                Ok::<(), ()>(())
            })
            .unwrap()
    );
    assert_eq!(checks, 2);
    assert!(!fixture::pidfd_ready(&pidfd));
    std::fs::write(directory.path().join("release"), b"release").unwrap();
    for index in 0..2 {
        fixture::wait_file(directory.path(), &format!("finished_{index}"));
    }
    fixture::finish(&mut child);
    assert!(fixture::pidfd_ready(&pidfd));
    assert_eq!(child.exit_code(), Some(0));
    fixture::reaped(pid);
    assert!(child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap());
    child.cleanup().unwrap();
}

#[test]
fn original_checkpoints_do_not_terminate_a_live_qualified_thread_group() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("threads", directory.path());
    let pid = fixture::identity(directory.path());
    fixture::wait_file(directory.path(), "main_leaving");
    fixture::main_zombie_with_threads(pid);
    child.request_control_close().unwrap();
    fixture::drain(&mut child);
    let mut checks = 0;
    let error = child
        .poll_normal_exit(|| {
            checks += 1;
            if checks == 2 {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "original-request-denied",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(checks, 2);
    assert!(
        matches!(error, ChildSpawnError::Checkpoint { primary, cleanup: None }
        if primary.kind() == io::ErrorKind::PermissionDenied && primary.to_string() == "original-request-denied")
    );
    fixture::advancing(directory.path(), 0);
    fixture::advancing(directory.path(), 1);
    std::fs::write(directory.path().join("release"), b"release").unwrap();
    fixture::finish(&mut child);
    fixture::reaped(pid);
}

#[test]
fn old_worker_entry_does_not_acquire_linux_normal_exit_permission() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = UnixChild::spawn_worker(
        &fixture::binary(),
        &[OsStr::new("exit"), directory.path().as_os_str()],
        &[],
        || Ok::<(), ()>(()),
    )
    .unwrap();
    fixture::wait_file(directory.path(), "ready");
    let pid = i32::try_from(fixture::scalar(directory.path(), "pid")).unwrap();
    child.request_control_close().unwrap();
    fixture::drain(&mut child);
    assert!(matches!(
        child.poll_normal_exit(|| Ok::<(), ()>(())),
        Err(ChildSpawnError::Operation(ChildError::Unsupported(_)))
    ));
    // 兼容负控只有旧异常 cleanup 收場，不把它当正常成功。
    child.cleanup().unwrap();
    fixture::reaped(pid);
}

#[test]
fn real_filter_installation_denial_preserves_os_error_and_executes_no_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(fixture::binary())
        .arg("deny_install")
        .arg(directory.path())
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(output.stdout.len() + output.stderr.len() < 65536);
    assert!(output.status.success(), "isolated host {output:?}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("DG_INNER_FILTER_DENIED=EPERM_NO_EXEC_NO_CHILD")
    );
    assert!(!directory.path().join("ready").exists());
}

#[cfg(target_arch = "x86_64")]
#[test]
fn x32_syscall_high_bit_is_rejected_even_when_audit_arch_is_x86_64() {
    rejects_wrong_abi("x32");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn actual_int80_compat_abi_is_rejected_before_any_compat_syscall_runs() {
    rejects_wrong_abi("compat");
}

#[cfg(target_arch = "x86_64")]
fn rejects_wrong_abi(mode: &str) {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn(mode, directory.path());
    let pid = fixture::identity(directory.path());
    fixture::wait_file(directory.path(), "about_to_call");
    child.request_control_close().unwrap();
    fixture::drain(&mut child);
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe {
            libc::waitid(
                libc::P_PID,
                pid as u32,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        },
        0
    );
    assert_eq!(unsafe { info.si_pid() }, pid);
    assert!(info.si_code == libc::CLD_KILLED || info.si_code == libc::CLD_DUMPED);
    assert_eq!(unsafe { info.si_status() }, libc::SIGSYS);
    assert!(!directory.path().join("after_wrong_abi").exists());
    fixture::finish(&mut child);
    assert_eq!(child.exit_code(), None);
    fixture::reaped(pid);
}

#[test]
fn atomic_birth_existing_scm_scanner_positive_is_only_compatibility_evidence() {
    let output = fixture::existing_scanner_route(&AtomicBirthFixture::new());
    let report: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(report["image"], 1);
    // 当前真实入口仍为 SCM；该案没有覆盖其首 receipt 之前的原子身份缺口。
}
