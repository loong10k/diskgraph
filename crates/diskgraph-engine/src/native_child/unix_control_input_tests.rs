//! A Unix 控制输入真实 OS 契约；不把 stdin EOF/leader wait 宣称为整组正常退场。

use super::unix_control_fixture::isolated_host;
use super::unix_control_test_support::{
    assert_reaped, command, complete, line, spawn, wait_file, write_bytes,
};
use super::{ChildError, ChildInputMode, ChildSpawnError, ControlWriteStatus, UnixChild};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

#[test]
fn binary_fragments_reach_child_stdin_then_close_delivers_real_eof() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = spawn("echo", temp.path());
    assert!(matches!(
        child.poll_control_write(),
        Err(ChildError::Unsupported(_))
    ));
    assert!(matches!(
        child.start_control_write(&[]),
        Err(ChildError::Unsupported(_))
    ));
    assert!(matches!(
        child.start_control_write(&vec![1; ControlWriteStatus::MAX_CHUNK_BYTES + 1]),
        Err(ChildError::Unsupported(_))
    ));
    let bytes: Vec<_> = (0..6145).map(|index| (index % 256) as u8).collect();
    let checkpoints = write_bytes(&mut child, &bytes);
    assert!(checkpoints >= 2);
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    assert_eq!(
        child.poll_control_write().unwrap(),
        ControlWriteStatus::Closed
    );
    assert!(matches!(
        child.start_control_write(b"late"),
        Err(ChildError::Unsupported(_))
    ));
    let output = complete(&mut child, temp.path());
    assert_eq!(line(&output, "DG_CONTROL_HEX="), hex::encode(&bytes));
    assert_eq!(std::fs::read(temp.path().join("received")).unwrap(), bytes);
    assert_eq!(line(&output, "DG_CONTROL_EOF="), "true");
}

#[test]
fn real_backpressure_keeps_one_owned_chunk_and_pending_only_polls() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = spawn("blocked", temp.path());
    wait_file(&temp.path().join("ready"));
    let mut bytes = [0_u8; ControlWriteStatus::MAX_CHUNK_BYTES];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
    let original = bytes;
    let mut expected = Sha256::new();
    let mut total = 0;
    let mut reached_pending = false;
    for _ in 0..4096 {
        match child.start_control_write(&bytes).unwrap() {
            ControlWriteStatus::Written(count) => {
                assert!(count > 0 && count <= bytes.len());
                expected.update(&bytes[..count]);
                total += count;
            }
            ControlWriteStatus::Pending => {
                reached_pending = true;
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(reached_pending, "must observe actual socket backpressure");
    assert!(!child.poll().unwrap());
    // 不读 stdin 的 live child 已造成真 Pending；修改源 slice 不得改变 owned chunk。
    bytes.fill(255);
    assert!(matches!(
        child.start_control_write(&bytes),
        Err(ChildError::Unsupported(_))
    ));
    assert_eq!(
        child.poll_control_write().unwrap(),
        ControlWriteStatus::Pending
    );
    std::fs::write(temp.path().join("release"), b"release").unwrap();
    let started = Instant::now();
    let written = loop {
        match child.poll_control_write().unwrap() {
            ControlWriteStatus::Pending => {
                assert!(started.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(1));
            }
            ControlWriteStatus::Written(count) => break count,
            other => panic!("unexpected pending result {other:?}"),
        }
    };
    assert!(written > 0 && written <= original.len());
    expected.update(&original[..written]);
    total += written;
    write_bytes(&mut child, &original[written..]);
    expected.update(&original[written..]);
    total += original.len() - written;
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    let output = complete(&mut child, temp.path());
    assert_eq!(line(&output, "DG_CONTROL_COUNT="), total.to_string());
    assert_eq!(
        line(&output, "DG_CONTROL_SHA256="),
        hex::encode(expected.finalize())
    );
    assert_eq!(line(&output, "DG_CONTROL_EOF="), "true");
}

#[test]
fn close_discards_only_unsent_pending_chunk_and_child_receives_eof() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = spawn("blocked", temp.path());
    wait_file(&temp.path().join("ready"));
    let bytes = [129; ControlWriteStatus::MAX_CHUNK_BYTES];
    let mut expected = Sha256::new();
    let mut total = 0;
    let mut pending = false;
    for _ in 0..4096 {
        match child.start_control_write(&bytes).unwrap() {
            ControlWriteStatus::Written(count) => {
                assert!(count > 0 && count <= bytes.len());
                expected.update(&bytes[..count]);
                total += count;
            }
            ControlWriteStatus::Pending => {
                pending = true;
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        pending,
        "pending qualification requires actual backpressure"
    );
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    assert_eq!(
        child.poll_control_write().unwrap(),
        ControlWriteStatus::Closed
    );
    std::fs::write(temp.path().join("release"), b"release").unwrap();
    let output = complete(&mut child, temp.path());
    assert_eq!(line(&output, "DG_CONTROL_COUNT="), total.to_string());
    assert_eq!(
        line(&output, "DG_CONTROL_SHA256="),
        hex::encode(expected.finalize())
    );
    assert_eq!(line(&output, "DG_CONTROL_EOF="), "true");
}

#[test]
fn closed_peer_returns_real_broken_pipe_without_changing_host_sigpipe() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = spawn("peer_closed", temp.path());
    wait_file(&temp.path().join("closed"));
    assert!(!child.poll().unwrap());
    let error = child.start_control_write(&[0, 128, 255]).unwrap_err();
    let original = error.native_io_error().expect("original EPIPE I/O");
    assert_eq!(original.kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(original.raw_os_error(), Some(libc::EPIPE));
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    std::fs::write(temp.path().join("release"), b"release").unwrap();
    complete(&mut child, temp.path());
}

#[test]
fn isolated_default_sigpipe_host_survives_actual_peer_closed_write() {
    let temp = tempfile::tempdir().unwrap();
    // 只有外层隔离宿主恢复 SIG_DFL；测试 runner 与 FFI 宿主的全局策略不变。
    let mut command = isolated_host(temp.path());
    let mut host = UnixChild::spawn(&mut command, || Ok::<(), ()>(())).unwrap();
    let output = complete(&mut host, temp.path());
    assert_eq!(line(&output, "DG_SIGPIPE_SAFE="), "true");
}

#[test]
fn explicit_null_preserves_two_checkpoints_and_all_control_methods_are_unsupported() {
    let temp = tempfile::tempdir().unwrap();
    let mut checks = 0;
    let mut child = UnixChild::spawn_with_input(
        &mut command("echo", temp.path()),
        ChildInputMode::Null,
        || {
            checks += 1;
            Ok::<(), ()>(())
        },
    )
    .unwrap();
    assert_eq!(checks, 2);
    assert!(matches!(
        child.start_control_write(b"x"),
        Err(ChildError::Unsupported(_))
    ));
    assert!(matches!(
        child.poll_control_write(),
        Err(ChildError::Unsupported(_))
    ));
    assert!(matches!(
        child.request_control_close(),
        Err(ChildError::Unsupported(_))
    ));
    let output = complete(&mut child, temp.path());
    assert_eq!(line(&output, "DG_CONTROL_HEX="), "");
}

#[test]
fn worker_control_first_checkpoint_rejects_before_real_child_creation() {
    #[derive(Debug)]
    struct OriginalFailure(&'static str);

    let temp = tempfile::tempdir().unwrap();
    let mut checks = 0;
    let error = match UnixChild::spawn_with_input(
        &mut command("linger", temp.path()),
        ChildInputMode::WorkerControl,
        || {
            checks += 1;
            Err::<(), _>(OriginalFailure("original cancellation"))
        },
    ) {
        Ok(_) => panic!("first checkpoint must reject before child creation"),
        Err(error) => error,
    };
    assert_eq!(checks, 1);
    assert!(!temp.path().join("pid").exists());
    assert!(
        matches!(error, ChildSpawnError::Checkpoint { primary, cleanup: None }
        if primary.0 == "original cancellation")
    );
}

#[test]
fn dropping_pending_control_owner_reaps_the_actual_live_child() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = spawn("blocked", temp.path());
    wait_file(&temp.path().join("ready"));
    let bytes = [42; ControlWriteStatus::MAX_CHUNK_BYTES];
    let mut pending = false;
    for _ in 0..4096 {
        match child.start_control_write(&bytes).unwrap() {
            ControlWriteStatus::Written(count) => assert!(count > 0 && count <= bytes.len()),
            ControlWriteStatus::Pending => {
                pending = true;
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(pending, "drop must start from actual backpressure");
    assert!(!child.poll().unwrap());
    drop(child);
    assert_reaped(temp.path());
}

#[test]
fn worker_control_checkpoint_failure_keeps_original_non_clone_error_and_real_reap() {
    #[derive(Debug)]
    struct OriginalFailure(&'static str);

    for cleanup_fault in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut command = command("linger", temp.path());
        if cleanup_fault {
            command.env("DG_PROBE_CLEANUP_FAULT", "1");
        }
        let mut checks = 0;
        let error =
            match UnixChild::spawn_with_input(&mut command, ChildInputMode::WorkerControl, || {
                checks += 1;
                if checks == 2 {
                    wait_file(&temp.path().join("ready"));
                    Err(OriginalFailure("original permission denied"))
                } else {
                    Ok(())
                }
            }) {
                Ok(_) => panic!("original typed checkpoint must reject"),
                Err(error) => error,
            };
        assert_eq!(checks, 2);
        assert!(
            matches!(error, ChildSpawnError::Checkpoint { primary, cleanup }
            if primary.0 == "original permission denied" && cleanup.is_some() == cleanup_fault)
        );
        assert_reaped(temp.path());
    }
}
