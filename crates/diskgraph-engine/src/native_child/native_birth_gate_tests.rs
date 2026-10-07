//! 真实原pipe窗口与受控出生协调合同；来源：原生 Rust macOS CLI/MCP描述符边界。
use super::macos_native_pipes::{MacosNativePipes, set_raw_pipe_hook};
use super::native_birth_gate::{NativeBirthGate, set_wait_hook};
use super::{ChildSpawnError, UnixChild};
use crate::EngineError;
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

#[test]
fn real_raw_pipe_window_blocks_controlled_birth_until_cloexec_preparation_finishes() {
    assert!(matches!(
        MacosNativePipes::prepare(super::ChildInputMode::Null, &mut || Ok(())),
        Err(EngineError::Business(
            diskgraph_core::BusinessError::Unsupported
        ))
    ));
    let (wait_tx, wait_rx) = mpsc::channel();
    let (born_tx, born_rx) = mpsc::channel();
    let born_rx = Arc::new(Mutex::new(born_rx));
    let hook_born_rx = Arc::clone(&born_rx);
    let (done_tx, done_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        set_wait_hook(Box::new(move || {
            wait_tx.send(()).unwrap();
        }));
        let mut command = Command::new("/usr/bin/true");
        let mut child = UnixChild::spawn(&mut command, || Ok::<(), ()>(())).unwrap();
        born_tx.send(()).unwrap();
        child.cleanup().unwrap();
    });
    set_raw_pipe_hook(Box::new(move |fds| {
        // 实际普通pipe尚未复制/关闭，确认目标窗口存在而非mock flag。
        for fd in fds {
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC,
                0
            );
        }
        done_tx.send(()).unwrap();
        wait_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("controlled creator actually waits at same gate");
        assert!(matches!(
            hook_born_rx.lock().unwrap().try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }));
    let pipes =
        MacosNativePipes::prepare(super::ChildInputMode::WorkerControl, &mut || Ok(())).unwrap();
    drop(pipes);
    born_rx
        .lock()
        .unwrap()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    thread.join().unwrap();
}

#[test]
fn waiting_gate_preserves_non_clone_original_checkpoint_and_creates_no_process() {
    struct OriginalFailure(Box<[u8]>);
    let payload = b"original gate cancellation".to_vec().into_boxed_slice();
    let address = payload.as_ptr() as usize;
    let held = NativeBirthGate::acquire(&mut || Ok::<(), ()>(())).unwrap();
    let (wait_tx, wait_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        set_wait_hook(Box::new(move || wait_tx.send(()).unwrap()));
        let mut command = Command::new("/usr/bin/true");
        let mut payload = Some(payload);
        let mut checks = 0;
        let result = UnixChild::spawn_checked(&mut command, || {
            checks += 1;
            if checks == 1 {
                Ok(())
            } else {
                Err(OriginalFailure(payload.take().unwrap()))
            }
        });
        match result {
            Err(ChildSpawnError::Checkpoint {
                primary,
                cleanup: None,
            }) => {
                assert_eq!(primary.0.as_ptr() as usize, address);
                assert_eq!(&*primary.0, b"original gate cancellation");
            }
            Ok(mut child) => {
                child.cleanup().unwrap();
                panic!("must not create child while gate is occupied");
            }
            _ => panic!("original checkpoint was replaced"),
        }
    });
    wait_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    thread.join().unwrap();
    drop(held);
}

#[test]
fn gate_wait_observes_original_budget_error_without_restarting_request_window() {
    let held = NativeBirthGate::acquire(&mut || Ok::<(), ()>(())).unwrap();
    let thread = std::thread::spawn(|| {
        let error = NativeBirthGate::acquire(&mut || {
            Err::<(), EngineError>(diskgraph_core::BusinessError::BudgetExceeded.into())
        })
        .unwrap_err();
        assert!(matches!(
            error,
            EngineError::Business(diskgraph_core::BusinessError::BudgetExceeded)
        ));
    });
    thread.join().unwrap();
    drop(held);
}

#[test]
fn occupied_gate_expires_at_original_absolute_deadline() {
    let held = NativeBirthGate::acquire(&mut || Ok::<(), ()>(())).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_millis(20);
    let thread = std::thread::spawn(move || {
        let mut checks = 0;
        let error = NativeBirthGate::acquire(&mut || {
            checks += 1;
            if std::time::Instant::now() >= deadline {
                Err::<(), EngineError>(diskgraph_core::BusinessError::BudgetExceeded.into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(checks >= 1);
        assert!(matches!(
            error,
            EngineError::Business(diskgraph_core::BusinessError::BudgetExceeded)
        ));
    });
    thread.join().unwrap();
    drop(held);
}

#[test]
fn cancellation_at_gate_handoff_cannot_be_skipped_by_successful_acquisition() {
    let held = NativeBirthGate::acquire(&mut || Ok::<(), ()>(())).unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let (ack_tx, ack_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut first = true;
        let result = NativeBirthGate::acquire(&mut || {
            if first {
                first = false;
                release_tx.send(()).unwrap();
                ack_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                Ok(())
            } else {
                Err::<(), EngineError>(EngineError::Poisoned)
            }
        });
        assert!(matches!(result, Err(EngineError::Poisoned)));
    });
    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(held);
    ack_tx.send(()).unwrap();
    thread.join().unwrap();
}

#[test]
fn raw_pipe_panic_closes_original_descriptors_and_poison_does_not_break_next_birth() {
    const MARKER: &str = "DISKGRAPH_RAW_PIPE_ISOLATED_TEST";
    if std::env::var_os(MARKER).is_none() {
        run_isolated_pipe_panic_test(MARKER);
        return;
    }
    // 原数字 FD 的 EBADF 只在独立单测试进程中核验；并行测试可复用已关闭的数字。
    let captured = Arc::new(Mutex::new(None));
    let observer = Arc::clone(&captured);
    set_raw_pipe_hook(Box::new(move |fds| {
        *observer.lock().unwrap() = Some(fds);
        std::panic::panic_any(String::from("original pipe preparation panic"));
    }));
    let panic = std::panic::catch_unwind(|| {
        let _ = MacosNativePipes::prepare(super::ChildInputMode::WorkerControl, &mut || Ok(()));
    })
    .unwrap_err();
    assert_eq!(
        *panic.downcast::<String>().unwrap(),
        "original pipe preparation panic"
    );
    for fd in captured.lock().unwrap().take().unwrap() {
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EBADF)
        );
    }
    let mut command = Command::new("/usr/bin/true");
    let mut child = UnixChild::spawn(&mut command, || Ok::<(), ()>(())).unwrap();
    child.cleanup().unwrap();
}

#[test]
fn contended_legacy_birth_keeps_postbirth_checkpoint_after_actual_creation() {
    use super::ChildInputMode;
    use super::unix_control_test_support::{command, wait_file};
    use std::sync::atomic::{AtomicBool, Ordering};
    let held = NativeBirthGate::acquire(&mut || Ok::<(), ()>(())).unwrap();
    let released = Arc::new(AtomicBool::new(false));
    let child_released = Arc::clone(&released);
    let (waiting_tx, waiting_rx) = mpsc::channel();
    let (phase_tx, phase_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let temp = tempfile::tempdir().unwrap();
        set_wait_hook(Box::new(move || waiting_tx.send(()).unwrap()));
        let mut checks = 0;
        let result = UnixChild::spawn_with_input(
            &mut command("linger", temp.path()),
            ChildInputMode::WorkerControl,
            || {
                checks += 1;
                if checks == 2 {
                    let after_release = child_released.load(Ordering::Acquire);
                    if after_release {
                        wait_file(&temp.path().join("ready"));
                    }
                    phase_tx
                        .send((checks, after_release, temp.path().join("ready").exists()))
                        .unwrap();
                    Err("original postbirth rejection")
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(
            result,
            Err(ChildSpawnError::Checkpoint {
                primary: "original postbirth rejection",
                ..
            })
        ));
        checks
    });
    waiting_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let early = phase_rx.recv_timeout(Duration::from_millis(100)).ok();
    released.store(true, Ordering::Release);
    drop(held);
    let checks = thread.join().unwrap();
    let observation = early.or_else(|| phase_rx.recv_timeout(Duration::from_secs(10)).ok());
    assert_eq!(checks, 2);
    assert_eq!(
        observation,
        Some((2, true, true)),
        "legacy postbirth callback ran while waiting before birth"
    );
}

#[test]
fn contended_admission_rejection_does_not_advance_lifecycle_callback() {
    use super::ChildInputMode;
    use super::unix_control_test_support::command;
    let held = NativeBirthGate::acquire(&mut || Ok::<(), ()>(())).unwrap();
    let thread = std::thread::spawn(|| {
        let temp = tempfile::tempdir().unwrap();
        let mut phases = 0;
        let result = UnixChild::spawn_with_input_and_admission(
            &mut command("linger", temp.path()),
            ChildInputMode::WorkerControl,
            || Err("original admission cancellation"),
            || {
                phases += 1;
                Ok(())
            },
        );
        assert!(matches!(
            result,
            Err(ChildSpawnError::Checkpoint {
                primary: "original admission cancellation",
                cleanup: None,
            })
        ));
        assert_eq!(
            phases, 1,
            "admission cancellation advanced postbirth lifecycle"
        );
        assert!(!temp.path().join("pid").exists());
    });
    thread.join().unwrap();
    drop(held);
}

// 子进程保持原 panic 清理与后续出生断言，不用宽松身份判断替代真实关闭。
fn run_isolated_pipe_panic_test(marker: &str) {
    struct OriginalChild(std::process::Child);
    impl Drop for OriginalChild {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let diagnostic = tempfile::tempfile().unwrap();
    let mut child = OriginalChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_child::native_birth_gate_tests::raw_pipe_panic_closes_original_descriptors_and_poison_does_not_break_next_birth",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(marker, "1")
            .stdin(std::process::Stdio::null())
            .stdout(diagnostic.try_clone().unwrap())
            .stderr(diagnostic.try_clone().unwrap())
            .spawn().unwrap(),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "isolated pipe test timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    // 限制失败诊断读取；父进程不依赖公共管道 EOF 来等待子进程。
    use std::io::{Read, Seek};
    let mut diagnostic = diagnostic;
    diagnostic.rewind().unwrap();
    let mut message = String::new();
    diagnostic
        .take(16 * 1024)
        .read_to_string(&mut message)
        .unwrap();
    assert!(
        status.success(),
        "isolated pipe panic regression failed: {message}"
    );
    assert!(
        message.contains("test result: ok. 1 passed; 0 failed;"),
        "isolated child did not execute exactly one successful regression: {message}"
    );
}
