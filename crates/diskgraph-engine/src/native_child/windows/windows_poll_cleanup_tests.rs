//! 有限清理API的真实Windows资格；编译缺API仅为开发RED，不是原生行为RED。
use super::cleanup_progress::CleanupProgress;
use super::overlapped_control_pipe::OverlappedControlPipe;
use super::overlapped_pipe::OverlappedPipe;
use super::pipe_security::PipeSecurity;
use super::windows_cleanup_rescue::WindowsCleanupRescue;
use super::windows_control_fixture::command;
use crate::native_child::{ChildInputMode, ControlWriteStatus};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    ERROR_IO_INCOMPLETE, ERROR_OPERATION_ABORTED, GetLastError, HANDLE,
};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};

fn pending(witness: (HANDLE, *const OVERLAPPED, *const u8)) -> (usize, usize) {
    let mut transferred = 0;
    let result = unsafe { GetOverlappedResult(witness.0, witness.1, &mut transferred, 0) };
    let error = if result == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    assert_eq!((result, error), (0, ERROR_IO_INCOMPLETE));
    (witness.1 as usize, witness.2 as usize)
}

#[test]
fn expired_read_cleanup_keeps_pending_storage_until_other_thread_completes() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let name: Vec<u16> = format!(r"\\.\pipe\diskgraph-poll-read-{}", uuid::Uuid::new_v4())
        .encode_utf16()
        .chain([0])
        .collect();
    let security = PipeSecurity::for_current_user().unwrap();
    // 准备责任先留在本例外槽；连接只FALSE轮询并消费原测试期限。
    let mut pipe_owner = None;
    OverlappedPipe::prepare_into(&name, &security, &mut pipe_owner).unwrap();
    let peer = OverlappedPipe::open_writer(&name, &security).unwrap();
    let connecting = pipe_owner.as_mut().unwrap();
    connecting.start_connect().unwrap();
    while !connecting.connect_ready().unwrap() {
        assert!(
            Instant::now() < deadline,
            "original pipe connection deadline"
        );
        std::thread::yield_now();
    }
    let mut pipe = pipe_owner.take().unwrap();
    let prepared = catch_unwind(AssertUnwindSafe(|| {
        assert!(pipe.read_next().unwrap().is_none());
        let addresses = pending(pipe.io_witness().unwrap());
        assert_eq!(
            pipe.poll_cleanup(Instant::now()).unwrap(),
            CleanupProgress::Pending
        );
        assert_eq!(pending(pipe.io_witness().unwrap()), addresses);
        addresses
    }));
    let addresses = match prepared {
        Ok(value) => value,
        Err(payload) => {
            let _ = pipe.cancel_pending();
            resume_unwind(payload);
        }
    };
    // 发起I/O的线程A在join期间保持存活；B在同一原期限内亲自取消并观察完成。
    let worker = std::thread::spawn(move || {
        let observed = catch_unwind(AssertUnwindSafe(|| {
            let (handle, operation, _) = pipe.io_witness().unwrap();
            assert_eq!(pending(pipe.io_witness().unwrap()), addresses);
            loop {
                assert!(
                    Instant::now() < deadline,
                    "original cleanup deadline exceeded"
                );
                if pipe.poll_cleanup(deadline).unwrap() == CleanupProgress::Complete {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let mut transferred = 0;
            let result = unsafe { GetOverlappedResult(handle, operation, &mut transferred, 0) };
            let error = if result == 0 {
                unsafe { GetLastError() }
            } else {
                0
            };
            assert_eq!((result, error), (0, ERROR_OPERATION_ABORTED));
            assert!(pipe.io_witness().is_err());
        }));
        let cleanup = pipe.cancel_pending();
        (pipe, observed, cleanup)
    });
    let (_pipe, observed, cleanup) = worker.join().unwrap();
    drop(peer);
    cleanup.unwrap();
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn expired_write_cleanup_keeps_pending_storage_until_other_thread_completes() {
    let deadline = Instant::now() + Duration::from_secs(10);
    // 原 owner 在连接提交前进入外槽，准备期间也消费本例原期限。
    let mut pipe_owner = None;
    let peer = OverlappedControlPipe::prepare_input_into(
        ChildInputMode::WorkerControl,
        &uuid::Uuid::new_v4(),
        &PipeSecurity::for_current_user().unwrap(),
        &mut pipe_owner,
        &mut || {
            if Instant::now() >= deadline {
                Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    let mut pipe = pipe_owner.take().unwrap();
    let prepared = catch_unwind(AssertUnwindSafe(|| {
        for _ in 0..128 {
            assert!(Instant::now() < deadline);
            if pipe
                .start_write(&[0x55; ControlWriteStatus::MAX_CHUNK_BYTES])
                .unwrap()
                == ControlWriteStatus::Pending
            {
                let addresses = pending(pipe.io_witness().unwrap());
                assert_eq!(
                    pipe.poll_cleanup(Instant::now()).unwrap(),
                    CleanupProgress::Pending
                );
                assert_eq!(pending(pipe.io_witness().unwrap()), addresses);
                return addresses;
            }
        }
        panic!("actual bounded write pressure did not create pending I/O");
    }));
    let addresses = match prepared {
        Ok(value) => value,
        Err(payload) => {
            let _ = pipe.cleanup();
            resume_unwind(payload);
        }
    };
    let worker = std::thread::spawn(move || {
        let observed = catch_unwind(AssertUnwindSafe(|| {
            assert_eq!(pending(pipe.io_witness().unwrap()), addresses);
            loop {
                assert!(
                    Instant::now() < deadline,
                    "original cleanup deadline exceeded"
                );
                if pipe.poll_cleanup(deadline).unwrap() == CleanupProgress::Complete {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(
                pipe.completion_error_for_test(),
                Some(ERROR_OPERATION_ABORTED)
            );
            assert!(pipe.io_witness().is_err());
        }));
        let cleanup = pipe.cleanup();
        (pipe, observed, cleanup)
    });
    let (_pipe, observed, cleanup) = worker.join().unwrap();
    drop(peer);
    cleanup.unwrap();
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn expired_child_cleanup_keeps_original_handles_until_native_complete() {
    let directory = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut child = crate::native_child::WindowsTestBirth::spawn(
        &mut command("hold", directory.path()),
        || Ok::<(), ()>(()),
    )
    .unwrap();
    let guardian = WindowsCleanupRescue::capture(&child).unwrap();
    let observed = catch_unwind(AssertUnwindSafe(|| {
        assert_eq!(
            child.poll_cleanup(Instant::now()).unwrap(),
            CleanupProgress::Pending
        );
        assert_eq!(child.cleanup_state_for_test(), (true, true, false));
        assert!(!guardian.waited().unwrap());
        assert!(guardian.active().unwrap() > 0);
        loop {
            assert!(
                Instant::now() < deadline,
                "original cleanup deadline exceeded"
            );
            if child.poll_cleanup(deadline).unwrap() == CleanupProgress::Complete {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(child.cleanup_state_for_test(), (false, false, true));
        assert!(guardian.waited().unwrap());
        assert_eq!(guardian.active().unwrap(), 0);
    }));
    // 失败路径沿既有安全兼容清理；不把其可能阻塞性质称作新有限API证明。
    let cleanup = child.cleanup();
    let rescued = guardian.finish();
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
    cleanup.unwrap();
    rescued.unwrap();
}

fn native_observation_failure(stage: u8) {
    use super::windows_cleanup_hooks::WindowsCleanupHooks as Hooks;
    let directory = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut child = crate::native_child::WindowsTestBirth::spawn(
        &mut command("hold", directory.path()),
        || Ok::<(), ()>(()),
    )
    .unwrap();
    let guardian = WindowsCleanupRescue::capture(&child).unwrap();
    Hooks::arm(stage);
    let observed = catch_unwind(AssertUnwindSafe(|| {
        let error = child.poll_cleanup(deadline).unwrap_err();
        assert_eq!(error.native_io_error().unwrap().raw_os_error(), Some(5));
        assert_eq!(child.cleanup_state_for_test(), (true, true, false));
        let before = Hooks::counts();
        assert_eq!(
            before.2, 1,
            "injection is a boundary observation failure, not native denial"
        );
        loop {
            assert!(
                Instant::now() < deadline,
                "original retry deadline exhausted"
            );
            if child.poll_cleanup(deadline).unwrap() == CleanupProgress::Complete {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let after = Hooks::counts();
        assert!(
            after.0 > before.0 && after.1 > before.1,
            "completion requires actual original wait and Job query"
        );
        assert_eq!(child.cleanup_state_for_test(), (false, false, true));
        assert!(guardian.waited().unwrap());
        assert_eq!(guardian.active().unwrap(), 0);
    }));
    Hooks::disarm();
    let cleanup = child.cleanup();
    let rescued = guardian.finish();
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
    cleanup.unwrap();
    rescued.unwrap();
}

#[test]
fn poll_cleanup_wait_failure_retains_original_owner_until_actual_retry() {
    native_observation_failure(1);
}

#[test]
fn poll_cleanup_query_failure_retains_original_owner_until_actual_retry() {
    native_observation_failure(2);
}
