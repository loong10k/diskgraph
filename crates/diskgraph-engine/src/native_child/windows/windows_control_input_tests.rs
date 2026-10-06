//! Windows 控制输入的真实 OS 阶段测试；不以字段注入或超时救援代替 pending／退出事实。

use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    ERROR_IO_INCOMPLETE, GetHandleInformation, GetLastError, HANDLE_FLAG_INHERIT, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::IO::GetOverlappedResult;
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::super::{ChildError, ChildInputMode, ChildSpawnError, ControlWriteStatus};
use super::owned_handle::OwnedHandle;
use super::windows_child::WindowsChild;
use super::windows_control_fixture::command;
use super::windows_control_test_witness::WindowsControlTestWitness;

fn check(deadline: Instant) -> Result<(), std::io::Error> {
    if Instant::now() >= deadline {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "original control test deadline",
        ))
    } else {
        Ok(())
    }
}

fn wait_until(deadline: Instant, mut condition: impl FnMut() -> bool) -> Result<(), ChildError> {
    while !condition() {
        check(deadline).map_err(|error| ChildError::io("control test wait", error))?;
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn collect_exit(
    child: &mut WindowsChild,
    deadline: Instant,
) -> Result<(Vec<u8>, Vec<u8>), ChildError> {
    let mut output = (Vec::new(), Vec::new());
    loop {
        check(deadline).map_err(|error| ChildError::io("control test exit", error))?;
        if let Some(bytes) = child.read_stdout()? {
            output.0.extend_from_slice(bytes);
        }
        if let Some(bytes) = child.read_stderr()? {
            output.1.extend_from_slice(bytes);
        }
        if output.0.len() + output.1.len() > 64 * 1024 {
            return Err(ChildError::Unsupported("fixture output limit"));
        }
        if child.poll()? && child.stdout_eof() && child.stderr_eof() {
            return Ok(output);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn pending_chunk(
    child: &mut WindowsChild,
    deadline: Instant,
) -> Result<(Vec<u8>, Vec<u8>), ChildError> {
    let mut completed = Vec::new();
    for index in 0..128u8 {
        let mut bytes: Vec<u8> = (0..ControlWriteStatus::MAX_CHUNK_BYTES)
            .map(|n| (n as u8) ^ index)
            .collect();
        while !bytes.is_empty() {
            check(deadline).map_err(|error| ChildError::io("control test pressure", error))?;
            match child.start_control_write(&bytes)? {
                ControlWriteStatus::Written(n) if n > 0 && n <= bytes.len() => {
                    completed.extend_from_slice(&bytes[..n]);
                    bytes.drain(..n);
                }
                ControlWriteStatus::Pending => return Ok((completed, bytes)),
                _ => {
                    return Err(ChildError::Unsupported(
                        "pressure did not report a valid actual write",
                    ));
                }
            }
        }
    }
    Err(ChildError::Unsupported(
        "no actual pending within fixed pressure bound",
    ))
}

fn incomplete(child: &WindowsChild) -> Result<(usize, usize), ChildError> {
    let (handle, operation, bytes) = child.control_io_witness_for_test()?;
    let mut transferred = 0;
    let result = unsafe { GetOverlappedResult(handle, operation, &mut transferred, 0) };
    let error = if result == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    if result != 0 || error != ERROR_IO_INCOMPLETE {
        return Err(ChildError::Unsupported(
            "actual write was not IO_INCOMPLETE",
        ));
    }
    Ok((operation as usize, bytes as usize))
}

#[test]
fn control_pending_keeps_its_owned_bytes_and_addresses_after_source_mutation_and_owner_move() {
    let directory = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut child = crate::native_child::WindowsTestBirth::spawn_with_input(
        &mut command("hold", directory.path()),
        ChildInputMode::WorkerControl,
        || check(deadline),
    )
    .unwrap();
    let result = (|| -> Result<_, ChildError> {
        wait_until(deadline, || directory.path().join("ready").exists())?;
        let (mut expected, mut source) = pending_chunk(&mut child, deadline)?;
        let original = source.clone();
        let addresses = incomplete(&child)?;
        source.fill(0xa7);
        drop(source);
        let mut moved = Box::new(child);
        let stable = incomplete(&moved)? == addresses;
        let pending_rejects_start = moved.start_control_write(b"must-not-overwrite").is_err();
        std::fs::write(directory.path().join("release"), b"release").unwrap();
        let written = loop {
            check(deadline).map_err(|error| ChildError::io("control test completion", error))?;
            match moved.poll_control_write()? {
                ControlWriteStatus::Pending => std::thread::sleep(Duration::from_millis(1)),
                ControlWriteStatus::Written(n) if n > 0 && n <= original.len() => break n,
                _ => return Err(ChildError::Unsupported("pending payload did not complete")),
            }
        };
        expected.extend_from_slice(&original[..written]);
        wait_until(deadline, || {
            std::fs::metadata(directory.path().join("received"))
                .is_ok_and(|m| m.len() == expected.len() as u64)
        })?;
        let closed = moved.request_control_close()?;
        let output = collect_exit(&mut moved, deadline)?;
        let exit = moved.exit_code();
        let cleanup = moved.cleanup();
        Ok((
            stable,
            pending_rejects_start,
            closed,
            output,
            exit,
            expected,
            cleanup,
        ))
    })();
    // child 已 move 入闭包；任何前置失败均由该真实 owner 的 Drop 终止并回收。
    let (stable, rejected, closed, output, exit, expected, cleanup) = result.unwrap();
    assert!(cleanup.is_ok(), "{cleanup:?}");
    assert!(stable && rejected);
    assert_eq!(closed, ControlWriteStatus::Closed);
    assert_eq!(exit, Some(0));
    assert!(directory.path().join("eof").exists());
    assert_eq!(
        std::fs::read(directory.path().join("received")).unwrap(),
        expected
    );
    assert!(output.0.windows(14).any(|b| b == b"control-stdout"));
    assert!(output.1.windows(14).any(|b| b == b"control-stderr"));
}

#[test]
fn close_seals_new_writes_and_retains_the_pending_operation_until_actual_completion() {
    let directory = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut child = crate::native_child::WindowsTestBirth::spawn_with_input(
        &mut command("hold", directory.path()),
        ChildInputMode::WorkerControl,
        || check(deadline),
    )
    .unwrap();
    let result = (|| -> Result<_, ChildError> {
        wait_until(deadline, || directory.path().join("ready").exists())?;
        let (_, source) = pending_chunk(&mut child, deadline)?;
        let addresses = incomplete(&child)?;
        let requested = child.request_control_close()?;
        let (handle, operation, bytes) = child.control_io_witness_for_test()?;
        let retained = (operation as usize, bytes as usize) == addresses;
        let mut flags = 0;
        let valid_while_pending = unsafe { GetHandleInformation(handle, &mut flags) } != 0;
        let rejects = child.start_control_write(b"after-close").is_err();
        let repeated = child.request_control_close()?;
        let mut last_written = None;
        loop {
            check(deadline).map_err(|error| ChildError::io("control test cancellation", error))?;
            match child.poll_control_write()? {
                ControlWriteStatus::Pending => std::thread::sleep(Duration::from_millis(1)),
                ControlWriteStatus::Written(n) if n <= source.len() && last_written.is_none() => {
                    last_written = Some(n)
                }
                ControlWriteStatus::Closed => break,
                _ => {
                    return Err(ChildError::Unsupported(
                        "close reported invalid or repeated completion",
                    ));
                }
            }
        }
        let idempotent = child.request_control_close()? == ControlWriteStatus::Closed
            && child.poll_control_write()? == ControlWriteStatus::Closed;
        std::fs::write(directory.path().join("release"), b"release").unwrap();
        let output = collect_exit(&mut child, deadline)?;
        Ok((
            requested,
            repeated,
            retained,
            valid_while_pending,
            rejects,
            idempotent,
            output,
            child.exit_code(),
        ))
    })();
    let _ = std::fs::write(directory.path().join("release"), b"release");
    let cleanup = child.cleanup();
    let (requested, repeated, retained, valid, rejects, idempotent, _, exit) = result.unwrap();
    assert!(cleanup.is_ok(), "{cleanup:?}");
    assert_eq!(requested, ControlWriteStatus::Pending);
    assert_eq!(repeated, ControlWriteStatus::Pending);
    assert!(retained && valid && rejects && idempotent);
    assert_eq!(exit, Some(0));
    assert!(directory.path().join("eof").exists());
}

#[test]
fn worker_control_uses_three_standard_pipe_handles_and_excludes_parent_events() {
    let directory = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let unlisted = OwnedHandle::from_raw(
        unsafe { CreateEventW(&attributes, 1, 0, null()) },
        "CreateEventW(unlisted control test)",
    )
    .unwrap();
    let mut command = command("inherit", directory.path());
    command.env(
        "DG_CONTROL_UNLISTED_EVENT",
        (unlisted.as_raw() as usize).to_string(),
    );
    let mut child = crate::native_child::WindowsTestBirth::spawn_with_input(
        &mut command,
        ChildInputMode::WorkerControl,
        || check(deadline),
    )
    .unwrap();
    // parent event 值只传测试 fixture；启动前未知该值，child 由文件门等待此安全元数据。
    let result = (|| -> Result<_, ChildError> {
        let rejects_idle = child.poll_control_write().is_err();
        let rejects_empty = child.start_control_write(b"").is_err();
        let oversized = vec![0; ControlWriteStatus::MAX_CHUNK_BYTES + 1];
        let rejects_oversized = child.start_control_write(&oversized).is_err();
        let (write, event) = child.control_handles_for_test()?;
        let mut write_flags = 0;
        let mut event_flags = 0;
        let noninheritable = unsafe { GetHandleInformation(write, &mut write_flags) } != 0
            && unsafe { GetHandleInformation(event, &mut event_flags) } != 0
            && write_flags & HANDLE_FLAG_INHERIT == 0
            && event_flags & HANDLE_FLAG_INHERIT == 0;
        std::fs::write(
            directory.path().join("parent-event"),
            (event as usize).to_string(),
        )
        .unwrap();
        wait_until(deadline, || directory.path().join("ready").exists())?;
        let payload = [0, 255, 1, 0, 17, 128];
        let mut cursor = 0;
        while cursor < payload.len() {
            check(deadline).map_err(|error| ChildError::io("control test payload", error))?;
            let mut status = child.start_control_write(&payload[cursor..])?;
            while status == ControlWriteStatus::Pending {
                check(deadline).map_err(|error| ChildError::io("control test poll", error))?;
                status = child.poll_control_write()?;
                std::thread::sleep(Duration::from_millis(1));
            }
            match status {
                ControlWriteStatus::Written(n) if n > 0 && n <= payload.len() - cursor => {
                    cursor += n
                }
                _ => {
                    return Err(ChildError::Unsupported(
                        "invalid fixture payload completion",
                    ));
                }
            }
        }
        wait_until(deadline, || {
            std::fs::metadata(directory.path().join("received"))
                .is_ok_and(|m| m.len() == payload.len() as u64)
        })?;
        let closed = child.request_control_close()?;
        let output = collect_exit(&mut child, deadline)?;
        Ok((
            noninheritable,
            rejects_idle && rejects_empty && rejects_oversized,
            closed,
            output,
            child.exit_code(),
            payload,
        ))
    })();
    let cleanup = child.cleanup();
    let (noninheritable, rejects, closed, _, exit, payload) = result.unwrap();
    assert!(cleanup.is_ok(), "{cleanup:?}");
    assert!(noninheritable && rejects);
    assert_eq!(closed, ControlWriteStatus::Closed);
    assert_eq!(exit, Some(0));
    assert_eq!(
        std::fs::read(directory.path().join("received")).unwrap(),
        payload
    );
    assert_eq!(
        unsafe { WaitForSingleObject(unlisted.as_raw(), 0) },
        WAIT_TIMEOUT
    );
}

#[test]
fn original_null_spawn_has_four_checkpoints_and_no_control_input() {
    let directory = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let witness = WindowsControlTestWitness::enable();
    let mut calls = 0;
    let mut child = crate::native_child::WindowsTestBirth::spawn(
        &mut command("null", directory.path()),
        || {
            calls += 1;
            check(deadline)
        },
    )
    .unwrap();
    let unsupported = matches!(
        child.start_control_write(b"x"),
        Err(ChildError::Unsupported(_))
    ) && matches!(child.poll_control_write(), Err(ChildError::Unsupported(_)))
        && matches!(
            child.request_control_close(),
            Err(ChildError::Unsupported(_))
        );
    let result = collect_exit(&mut child, deadline);
    let exit = child.exit_code();
    let cleanup = child.cleanup();
    let terminated = witness.terminated_and_empty();
    result.unwrap();
    assert!(cleanup.is_ok(), "{cleanup:?}");
    assert_eq!(calls, 4);
    assert!(unsupported);
    assert_eq!(exit, Some(0));
    assert_eq!(witness.creations(), 1);
    assert!(terminated.unwrap());
    assert_eq!(
        std::fs::read(directory.path().join("received")).unwrap(),
        b""
    );
}

#[test]
fn third_original_checkpoint_preserves_non_clone_primary_and_cleans_the_suspended_job() {
    let directory = tempfile::tempdir().unwrap();
    let witness = WindowsControlTestWitness::enable();
    let mut calls = 0;
    let result = crate::native_child::WindowsTestBirth::spawn(
        &mut command("stamp", directory.path()),
        || {
            calls += 1;
            if calls == 3 {
                Err(std::io::Error::other("original authority denied sentinel"))
            } else {
                Ok(())
            }
        },
    );
    let error = match result {
        Ok(mut child) => {
            child.cleanup().unwrap();
            panic!("third checkpoint allowed execution")
        }
        Err(error) => error,
    };
    assert_eq!(calls, 3);
    assert_eq!(witness.creations(), 1);
    assert!(witness.terminated_and_empty().unwrap());
    assert!(!directory.path().join("started").exists());
    assert!(
        matches!(error, ChildSpawnError::Checkpoint { primary, cleanup: None } if primary.to_string() == "original authority denied sentinel")
    );
}

#[test]
fn original_post_create_failure_cleans_actual_job_with_external_witness_handles_held() {
    let directory = tempfile::tempdir().unwrap();
    let witness = WindowsControlTestWitness::enable();
    let mut command = command("stamp", directory.path());
    command.env("DG_WINDOWS_NATIVE_FAULT", "post_create");
    let mut calls = 0;
    let result = crate::native_child::WindowsTestBirth::spawn(&mut command, || {
        calls += 1;
        Ok::<(), ()>(())
    });
    let error = match result {
        Ok(mut child) => {
            child.cleanup().unwrap();
            panic!("post-create failure was ignored")
        }
        Err(error) => error,
    };
    assert_eq!(calls, 2);
    assert_eq!(witness.creations(), 1);
    assert!(witness.terminated_and_empty().unwrap());
    assert!(!directory.path().join("started").exists());
    assert!(matches!(
        error,
        ChildSpawnError::Operation(ChildError::Unsupported("injected post-create failure"))
    ));
}
