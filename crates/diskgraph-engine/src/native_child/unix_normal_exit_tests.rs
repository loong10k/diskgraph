//! Unix 正常退出许可的六类真实资格与负控；来源：独占 child/session/native group 运行契约。

use super::unix_normal_exit_test_support as fixture;
use super::{ChildError, ChildSpawnError, ControlWriteStatus, UnixChild};
use std::io;
use std::process::Command;

thread_local! {
    static REAP_BEFORE_CLEANUP_WAIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static GROUP_TERMINATION_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 注入持续的组终止错误，保留真实 leader 以验证回收责任，不作为原生权限验收。
/// 参数：无；返回：本线程是否注入错误。
pub(super) fn group_termination_failure() -> bool {
    GROUP_TERMINATION_FAILURE.get()
}

/// 在真实清理末段消费原 leader；仅测试当前线程的单次外部等待竞态。
/// 参数：pid 为真实原 leader；返回：无，断言外部 waitpid 实际消费该 child。
pub(super) fn reap_before_cleanup_wait(pid: i32) {
    if REAP_BEFORE_CLEANUP_WAIT.replace(false) {
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    }
}

#[test]
fn group_termination_failure_retains_original_leader_and_capacity_until_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("exit", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    child.request_control_close().unwrap();
    fixture::drain(&mut child, true);
    fixture::retained(pid);
    GROUP_TERMINATION_FAILURE.set(true);
    let error = child.cleanup().unwrap_err();
    GROUP_TERMINATION_FAILURE.set(false);
    assert!(
        error
            .native_io_error()
            .is_some_and(|original| original.raw_os_error() == Some(libc::EPERM))
    );
    // 原始代码在 group 失败后仍 wait；该断言直接检测原 leader 已被错误回收。
    fixture::retained(pid);
    GROUP_TERMINATION_FAILURE.set(true);
    #[cfg(target_os = "macos")]
    {
        let registry = crate::scan_worker_registry::ScanWorkerRegistry::new(1).unwrap();
        let reservation = registry.reserve().unwrap();
        reservation.retain(child);
        drop(reservation);
        for _ in 0..3 {
            assert!(registry.drain().is_err());
            assert_eq!(registry.occupied().unwrap(), 1);
            assert!(registry.reserve().is_err());
            fixture::retained(pid);
        }
        GROUP_TERMINATION_FAILURE.set(false);
        registry.drain().unwrap();
        assert_eq!(registry.occupied().unwrap(), 0);
        drop(registry.reserve().unwrap());
    }
    #[cfg(not(target_os = "macos"))]
    {
        for _ in 0..3 {
            assert!(child.cleanup().is_err());
            fixture::retained(pid);
        }
        GROUP_TERMINATION_FAILURE.set(false);
        child.cleanup().unwrap();
    }
    fixture::reaped(pid);
}

#[test]
fn end_and_pipe_eof_do_not_permit_reaping_a_live_leader() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("live_end", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    child.request_control_close().unwrap();
    fixture::drain(&mut child, false);
    assert!(child.stdout_eof() && child.stderr_eof());
    assert!(!child.poll().unwrap());
    assert!(!child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap());
    fixture::advancing(directory.path());
    fixture::release(directory.path());
    fixture::finish(&mut child);
    assert_eq!(child.exit_code(), Some(0));
    fixture::reaped(pid);
}

#[test]
fn exited_retained_leader_and_eof_do_not_hide_a_live_ordinary_descendant() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("descendant", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    fixture::wait_file(&directory.path().join("descendant_ready"));
    let leaf = directory.path().join("leaf");
    assert_eq!(fixture::scalar(&leaf, "pgid"), pid);
    assert_eq!(fixture::scalar(&leaf, "sid"), pid);
    assert_eq!(fixture::scalar(&leaf, "ppid"), pid);
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    fixture::drain(&mut child, true);
    fixture::retained(pid);
    assert!(!child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap());
    fixture::retained(pid);
    fixture::advancing(&leaf);
    fixture::release(&leaf);
    assert!(fixture::finish(&mut child) >= 2);
    assert_eq!(child.exit_code(), Some(0));
    fixture::reaped(pid);
    // 成功后的缓存许可不再对已可复用的数值 PGID 操作；重复调用保持同一事实。
    assert!(child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap());
    child.cleanup().unwrap();
    fixture::reaped(pid);
}

#[test]
fn checkpoint_primary_is_non_clone_and_does_not_terminate_a_live_group() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("descendant", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    fixture::wait_file(&directory.path().join("descendant_ready"));
    child.request_control_close().unwrap();
    fixture::drain(&mut child, true);
    let error = child
        .poll_normal_exit(|| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "original-authority",
            ))
        })
        .unwrap_err();
    match error {
        ChildSpawnError::Checkpoint { primary, cleanup } => {
            assert_eq!(primary.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(primary.to_string(), "original-authority");
            assert!(cleanup.is_none());
        }
        other => panic!("wrong checkpoint outcome {other:?}"),
    }
    fixture::retained(pid);
    let leaf = directory.path().join("leaf");
    fixture::advancing(&leaf);
    fixture::release(&leaf);
    fixture::finish(&mut child);
    fixture::reaped(pid);
}

#[test]
fn later_checkpoint_error_does_not_consume_the_retained_leader() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("exit", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    child.request_control_close().unwrap();
    fixture::drain(&mut child, true);
    let mut calls = 0;
    let error = child
        .poll_normal_exit(|| {
            calls += 1;
            if calls == 2 {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "original-deadline",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(calls, 2);
    assert!(
        matches!(error, ChildSpawnError::Checkpoint { primary, cleanup: None } if primary.kind() == io::ErrorKind::Interrupted && primary.to_string() == "original-deadline")
    );
    fixture::retained(pid);
    fixture::finish(&mut child);
    fixture::reaped(pid);
}

#[test]
fn two_sessions_have_independent_normal_exit_permissions() {
    let live_directory = tempfile::tempdir().unwrap();
    let done_directory = tempfile::tempdir().unwrap();
    let mut live = fixture::spawn("descendant", live_directory.path());
    let live_pid = fixture::qualified_identity(live_directory.path());
    fixture::wait_file(&live_directory.path().join("descendant_ready"));
    live.request_control_close().unwrap();
    fixture::drain(&mut live, true);
    let mut done = fixture::spawn("nonzero", done_directory.path());
    let done_pid = fixture::qualified_identity(done_directory.path());
    assert_ne!(live_pid, done_pid);
    done.request_control_close().unwrap();
    fixture::drain(&mut done, true);
    assert!(!live.poll_normal_exit(|| Ok::<(), ()>(())).unwrap());
    fixture::finish(&mut done);
    assert_eq!(done.exit_code(), Some(7));
    fixture::reaped(done_pid);
    drop(done);
    let leaf = live_directory.path().join("leaf");
    fixture::retained(live_pid);
    fixture::advancing(&leaf);
    fixture::release(&leaf);
    fixture::finish(&mut live);
    fixture::reaped(live_pid);
}

#[test]
fn external_reap_loses_normal_permission_and_refuses_old_numeric_group_cleanup() {
    for late_reap in [false, true] {
        external_reap_refuses_cleanup_completion(late_reap);
    }
}

fn external_reap_refuses_cleanup_completion(late_reap: bool) {
    let directory = tempfile::tempdir().unwrap();
    let mut child = fixture::spawn("exit", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    child.request_control_close().unwrap();
    fixture::drain(&mut child, true);
    fixture::retained(pid);
    let mut status = 0;
    // 安全性：故意消费本测试 leader，验证 owner 丢失时禁止后续 PGID 授权。
    if late_reap {
        assert!(child.poll().unwrap());
        fixture::retained(pid);
        REAP_BEFORE_CLEANUP_WAIT.set(true);
        let error = child.cleanup().unwrap_err();
        assert!(
            error
                .native_io_error()
                .is_some_and(|original| original.raw_os_error() == Some(libc::ECHILD))
        );
    } else {
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
        assert!(libc::WIFEXITED(status));
        let error = child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap_err();
        assert!(
            matches!(
                &error,
                ChildSpawnError::Operation(ChildError::Unsupported(_))
            ) || matches!(&error, ChildSpawnError::Operation(native)
                if native.native_io_error().is_some_and(|original|
                    original.raw_os_error() == Some(libc::ECHILD)))
        );
    }
    for _ in 0..3 {
        assert!(
            matches!(child.cleanup(), Err(ChildError::Unsupported(_))),
            "lost ownership must never turn into successful cleanup on retry"
        );
    }
    #[cfg(target_os = "macos")]
    {
        let registry = crate::scan_worker_registry::ScanWorkerRegistry::new(1).unwrap();
        let reservation = registry.reserve().unwrap();
        reservation.retain(child);
        drop(reservation);
        for _ in 0..3 {
            assert!(
                registry.drain().is_err(),
                "unknown ownership released recovery slot"
            );
            assert_eq!(registry.occupied().unwrap(), 1);
            assert!(registry.reserve().is_err());
        }
        // 此夹具已由真实waitpid消费原leader；生产registry没有此额外事实，仍须保留失败。
    }
    fixture::reaped(pid);
}

#[test]
fn old_configured_command_has_no_qualified_normal_exit_permission() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "native_child::unix_normal_exit_fixture::normal_fixture",
            "--nocapture",
        ])
        .env("DG_NORMAL_FIXTURE", "exit")
        .env("DG_NORMAL_DIRECTORY", directory.path());
    let mut child = UnixChild::spawn(&mut command, || Ok::<(), ()>(())).unwrap();
    fixture::wait_file(&directory.path().join("ready"));
    let pid = fixture::scalar(directory.path(), "pid");
    // 旧 probe 同属宿主 session，不能因为管道结束/名字相同就提升为 qualified worker。
    assert_eq!(fixture::scalar(directory.path(), "sid"), unsafe {
        libc::getsid(0)
    });
    fixture::drain(&mut child, true);
    fixture::retained(pid);
    assert!(matches!(
        child.poll_normal_exit(|| Ok::<(), ()>(())),
        Err(ChildSpawnError::Operation(ChildError::Unsupported(_)))
    ));
    fixture::retained(pid);
    // 仅此兼容负控使用旧异常 cleanup 收场，不把它计作正常许可。
    child.cleanup().unwrap();
    fixture::reaped(pid);
}
