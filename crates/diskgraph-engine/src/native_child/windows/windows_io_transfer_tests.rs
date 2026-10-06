//! 真实pending管道跨线程交接；查询失败注入仅验证边界，不冒充内核拒权。
use super::overlapped_control_pipe::OverlappedControlPipe;
use super::overlapped_pipe::OverlappedPipe;
use super::owned_handle::OwnedHandle;
use super::pipe_security::PipeSecurity;
use crate::native_child::{ChildInputMode, ControlWriteStatus};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_IO_INCOMPLETE, ERROR_OPERATION_ABORTED,
    GetLastError, HANDLE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, WaitForSingleObject};

fn incomplete(witness: (HANDLE, *const OVERLAPPED, *const u8)) -> (usize, usize) {
    let (handle, operation, bytes) = witness;
    let mut transferred = 0;
    let result = unsafe { GetOverlappedResult(handle, operation, &mut transferred, 0) };
    let error = if result == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    assert_eq!(result, 0);
    assert_eq!(
        error, ERROR_IO_INCOMPLETE,
        "must observe actual OS pending before owner transfer"
    );
    (operation as usize, bytes as usize)
}

fn read_case(inject: bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let security = PipeSecurity::for_current_user().unwrap();
    let name: Vec<u16> = format!(r"\\.\pipe\diskgraph-read-transfer-{}", uuid::Uuid::new_v4())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
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
    // 唯一owner始终在catch外；无数据且peer仍打开，真实read必须pending。
    let prepared = catch_unwind(AssertUnwindSafe(|| {
        assert!(pipe.read_next().unwrap().is_none());
        let addresses = incomplete(pipe.io_witness().unwrap());
        if inject {
            OverlappedPipe::fail_next_query_for_test();
            let error = pipe.read_next().unwrap_err();
            assert_eq!(error.native_io_error().unwrap().raw_os_error(), Some(5));
            assert_eq!(incomplete(pipe.io_witness().unwrap()), addresses);
        }
        addresses
    }));
    let addresses = match prepared {
        Ok(addresses) => addresses,
        Err(payload) => {
            let _ = pipe.cancel_pending();
            resume_unwind(payload);
        }
    };
    // 移动整个pipe而非只传一个裸地址；所有权到B后仍有原稳定operation/buffer。
    let worker = std::thread::spawn(move || {
        let observed = catch_unwind(AssertUnwindSafe(|| {
            assert!(Instant::now() < deadline);
            let (handle, operation, _) = pipe.io_witness().unwrap();
            assert_eq!(incomplete(pipe.io_witness().unwrap()), addresses);
            pipe.cancel_pending().unwrap();
            let mut transferred = 0;
            let completed = unsafe { GetOverlappedResult(handle, operation, &mut transferred, 0) };
            let error = if completed == 0 {
                unsafe { GetLastError() }
            } else {
                0
            };
            assert_eq!((completed, error), (0, ERROR_OPERATION_ABORTED));
            assert!(Instant::now() < deadline);
            assert!(
                pipe.io_witness().is_err(),
                "actual completion consumed the pending read"
            );
            assert!(pipe.eof());
        }));
        let cleaned = pipe.cancel_pending();
        (pipe, observed, cleaned)
    });
    let (mut pipe, observed, cleaned) = worker.join().unwrap();
    let final_cleanup = pipe.cancel_pending();
    drop(peer);
    cleaned.unwrap();
    final_cleanup.unwrap();
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn pending_read_owner_moves_threads_with_stable_storage_and_cancellation() {
    read_case(false);
}

#[test]
fn pending_read_query_failure_keeps_kernel_memory_until_other_thread_cleanup() {
    read_case(true);
}

#[test]
fn pending_write_owner_moves_threads_with_stable_storage_and_cancellation() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let security = PipeSecurity::for_current_user().unwrap();
    // 原 owner 在连接提交前进入外槽，准备期间也消费本例原期限。
    let mut pipe_owner = None;
    let peer = OverlappedControlPipe::prepare_input_into(
        ChildInputMode::WorkerControl,
        &uuid::Uuid::new_v4(),
        &security,
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
        let block = [0x6a; ControlWriteStatus::MAX_CHUNK_BYTES];
        for _ in 0..128 {
            assert!(Instant::now() < deadline);
            match pipe.start_write(&block).unwrap() {
                ControlWriteStatus::Pending => return incomplete(pipe.io_witness().unwrap()),
                ControlWriteStatus::Written(n) => assert!(n > 0 && n <= block.len()),
                ControlWriteStatus::Closed => panic!("unexpected closed write"),
            }
        }
        panic!("fixed pressure must reach actual pending while reader remains idle");
    }));
    let addresses = match prepared {
        Ok(addresses) => addresses,
        Err(payload) => {
            let _ = pipe.cleanup();
            resume_unwind(payload);
        }
    };
    let worker = std::thread::spawn(move || {
        let observed = catch_unwind(AssertUnwindSafe(|| {
            assert!(Instant::now() < deadline);
            assert_eq!(incomplete(pipe.io_witness().unwrap()), addresses);
            pipe.cleanup().unwrap();
            assert_eq!(
                pipe.completion_error_for_test(),
                Some(ERROR_OPERATION_ABORTED)
            );
            assert!(Instant::now() < deadline);
            assert!(pipe.io_witness().is_err());
            assert_eq!(pipe.poll_write().unwrap(), ControlWriteStatus::Closed);
        }));
        let cleaned = pipe.cleanup();
        (pipe, observed, cleaned)
    });
    let (mut pipe, observed, cleaned) = worker.join().unwrap();
    let final_cleanup = pipe.cleanup();
    drop(peer);
    cleaned.unwrap();
    final_cleanup.unwrap();
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn pending_read_drop_on_receiving_thread_waits_for_real_completion() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let security = PipeSecurity::for_current_user().unwrap();
    let name: Vec<u16> = format!(r"\\.\pipe\diskgraph-read-drop-{}", uuid::Uuid::new_v4())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
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
        let witness = pipe.io_witness().unwrap();
        incomplete(witness);
        let event = unsafe { (*witness.1).hEvent };
        let mut duplicate = std::ptr::null_mut();
        let process = unsafe { GetCurrentProcess() };
        assert_ne!(
            unsafe {
                DuplicateHandle(
                    process,
                    event,
                    process,
                    &mut duplicate,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                )
            },
            0
        );
        let completion =
            OwnedHandle::from_raw(duplicate, "DuplicateHandle(read completion)").unwrap();
        // event在ReadFile前已reset，独立副本只观察该次真实完成，不消费operation内存。
        (completion, (witness.1 as usize, witness.2 as usize))
    }));
    let (completion, addresses) = match prepared {
        Ok(value) => value,
        Err(payload) => {
            let _ = pipe.cancel_pending();
            resume_unwind(payload);
        }
    };
    let joined = std::thread::spawn(move || {
        let observed = catch_unwind(AssertUnwindSafe(|| {
            assert!(Instant::now() < deadline);
            assert_eq!(incomplete(pipe.io_witness().unwrap()), addresses);
        }));
        drop(pipe);
        observed
    })
    .join()
    .unwrap();
    let waited = unsafe { WaitForSingleObject(completion.as_raw(), 0) };
    drop(peer);
    assert_eq!(
        waited, WAIT_OBJECT_0,
        "Drop must wait for the original read completion before freeing storage"
    );
    assert!(Instant::now() < deadline);
    if let Err(payload) = joined {
        resume_unwind(payload);
    }
}
