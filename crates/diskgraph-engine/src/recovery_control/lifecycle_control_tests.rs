use super::{ControlError, ControlFrame, ControlNotification, ControlReceiver};
use std::time::{Duration, Instant};

#[test]
fn explicit_lifecycle_version_accepts_completion() {
    let mut receiver = ControlReceiver::new_lifecycle([9; 32]);
    for (sequence, notification) in [
        ControlNotification::Ready {},
        ControlNotification::ForegroundEnded {
            exit_code: 0,
            panicked: false,
        },
        ControlNotification::CleanupPending {},
        ControlNotification::CleanupComplete {},
    ]
    .into_iter()
    .enumerate()
    {
        let frame = ControlFrame {
            version: 2,
            session: [9; 32],
            sequence: sequence as u64,
            notification,
        };
        assert_eq!(
            receiver.push(&frame.encode().unwrap()).unwrap(),
            vec![frame]
        );
    }
    receiver.end_of_stream().unwrap();
}

fn encoded(version: u16, sequence: u64, notification: ControlNotification) -> Vec<u8> {
    ControlFrame {
        version,
        sequence,
        session: [9; 32],
        notification,
    }
    .encode()
    .unwrap()
}

fn foreground() -> ControlNotification {
    ControlNotification::ForegroundEnded {
        exit_code: 0,
        panicked: false,
    }
}

#[test]
fn lifecycle_completes_without_pending_with_fragmented_input() {
    let mut receiver = ControlReceiver::new_lifecycle([9; 32]);
    for (sequence, notification) in [
        ControlNotification::Ready {},
        foreground(),
        ControlNotification::CleanupComplete {},
    ]
    .into_iter()
    .enumerate()
    {
        let bytes = encoded(2, sequence as u64, notification);
        for byte in bytes {
            receiver.push(&[byte]).unwrap();
        }
    }
    receiver.end_of_stream().unwrap();
    assert_eq!(
        receiver.push(&encoded(2, 3, ControlNotification::Ready {})),
        Err(ControlError::Protocol)
    );
    assert_eq!(receiver.end_of_stream(), Err(ControlError::Protocol));
}

#[test]
fn versions_are_not_interchangeable() {
    let mut legacy = ControlReceiver::new([9; 32], Instant::now() + Duration::from_secs(1));
    assert_eq!(
        legacy.push(&encoded(2, 0, ControlNotification::Ready {})),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        legacy.push(&encoded(1, 0, ControlNotification::Ready {})),
        Err(ControlError::Protocol)
    );
    let mut lifecycle = ControlReceiver::new_lifecycle([9; 32]);
    assert_eq!(
        lifecycle.push(&encoded(1, 0, ControlNotification::Ready {})),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        lifecycle.push(&encoded(2, 0, ControlNotification::Ready {})),
        Err(ControlError::Protocol)
    );
}

#[test]
fn legacy_expiry_cannot_be_renewed_by_lifecycle_input() {
    let mut legacy = ControlReceiver::new([9; 32], Instant::now() - Duration::from_secs(1));
    assert_eq!(
        legacy.push(&encoded(2, 0, ControlNotification::Ready {})),
        Err(ControlError::Deadline)
    );
    assert_eq!(legacy.end_of_stream(), Err(ControlError::Deadline));
}

#[test]
fn lifecycle_rejects_repeated_pending_without_waiting_for_budget() {
    let mut receiver = ControlReceiver::new_lifecycle([9; 32]);
    for (sequence, notification) in [
        ControlNotification::Ready {},
        foreground(),
        ControlNotification::CleanupPending {},
    ]
    .into_iter()
    .enumerate()
    {
        receiver
            .push(&encoded(2, sequence as u64, notification))
            .unwrap();
    }
    assert_eq!(
        receiver.push(&encoded(2, 3, ControlNotification::CleanupPending {})),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        receiver.push(&encoded(2, 3, ControlNotification::CleanupComplete {})),
        Err(ControlError::Protocol)
    );
}

#[test]
fn lifecycle_rejects_wrong_session_sequence_and_order() {
    for (session, sequence, notification) in [
        ([8; 32], 0, ControlNotification::Ready {}),
        ([9; 32], 1, ControlNotification::Ready {}),
        ([9; 32], 0, foreground()),
    ] {
        let mut receiver = ControlReceiver::new_lifecycle([9; 32]);
        let bytes = ControlFrame {
            version: 2,
            session,
            sequence,
            notification,
        }
        .encode()
        .unwrap();
        assert_eq!(receiver.push(&bytes), Err(ControlError::Protocol));
        assert_eq!(receiver.end_of_stream(), Err(ControlError::Protocol));
    }
}

#[test]
fn lifecycle_eof_requires_complete_and_no_partial_frame() {
    for input in [
        Vec::new(),
        encoded(2, 0, ControlNotification::Ready {}),
        vec![0, 0],
    ] {
        let mut receiver = ControlReceiver::new_lifecycle([9; 32]);
        receiver.push(&input).unwrap();
        assert_eq!(receiver.end_of_stream(), Err(ControlError::Unconfirmed));
        assert_eq!(
            receiver.push(&encoded(2, 0, ControlNotification::Ready {})),
            Err(ControlError::Unconfirmed)
        );
    }
}

#[test]
fn lifecycle_charges_invalid_input_before_allocating() {
    for bytes in [vec![0; 16401], 4097_u32.to_be_bytes().to_vec()] {
        let mut receiver = ControlReceiver::new_lifecycle([9; 32]);
        assert_eq!(receiver.push(&bytes), Err(ControlError::Budget));
        assert_eq!(receiver.end_of_stream(), Err(ControlError::Budget));
    }
}

#[test]
fn lifecycle_accepts_exact_cumulative_and_frame_byte_limits() {
    let mut bytes = Vec::new();
    for (sequence, notification) in [
        ControlNotification::Ready {},
        foreground(),
        ControlNotification::CleanupPending {},
        ControlNotification::CleanupComplete {},
    ]
    .into_iter()
    .enumerate()
    {
        let body = serde_json::to_vec(&ControlFrame {
            version: 2,
            sequence: sequence as u64,
            session: [9; 32],
            notification,
        })
        .unwrap();
        bytes.extend_from_slice(&4096_u32.to_be_bytes());
        bytes.extend_from_slice(&body);
        bytes.resize(bytes.len() + 4096 - body.len(), b' ');
    }
    assert_eq!(bytes.len(), 16400);
    let mut receiver = ControlReceiver::new_lifecycle([9; 32]);
    assert_eq!(receiver.push(&bytes).unwrap().len(), 4);
    receiver.end_of_stream().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn lifecycle_writer_uses_separate_finite_delivery_deadlines() {
    use super::UnixControlWriter;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicBool;
    let (sender, mut reader) = UnixStream::pair().unwrap();
    let mut writer = UnixControlWriter::new_lifecycle(sender, [9; 32]).unwrap();
    for notification in [
        ControlNotification::Ready {},
        foreground(),
        ControlNotification::CleanupComplete {},
    ] {
        writer
            .send_lifecycle(
                notification,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
    }
    drop(writer);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    let mut receiver = ControlReceiver::new_lifecycle([9; 32]);
    assert_eq!(receiver.push(&bytes).unwrap().len(), 3);
    receiver.end_of_stream().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn failed_lifecycle_delivery_cannot_be_retried_with_new_deadline() {
    use super::UnixControlWriter;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicBool;
    let (sender, _reader) = UnixStream::pair().unwrap();
    let mut writer = UnixControlWriter::new_lifecycle(sender, [9; 32]).unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        writer.send_lifecycle(
            ControlNotification::Ready {},
            &cancel,
            Instant::now() - Duration::from_secs(1)
        ),
        Err(ControlError::Deadline)
    );
    assert_eq!(
        writer.send_lifecycle(
            ControlNotification::Ready {},
            &cancel,
            Instant::now() + Duration::from_secs(1)
        ),
        Err(ControlError::Deadline)
    );
    assert_eq!(
        writer.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Deadline)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn blocked_lifecycle_writer_honors_delivery_deadline_and_never_retries() {
    use super::UnixControlWriter;
    use std::io::{ErrorKind, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicBool;
    let (mut sender, _reader) = UnixStream::pair().unwrap();
    sender.set_nonblocking(true).unwrap();
    // 先以真实内核 WouldBlock 填满同一通道；不把生产通知发送成功误当期限验证。
    loop {
        match sender.write(&[0; 4096]) {
            Ok(count) => assert!(count > 0),
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => panic!("socket setup: {error}"),
        }
    }
    let mut writer = UnixControlWriter::new_lifecycle(sender, [9; 32]).unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        writer.send_lifecycle(
            ControlNotification::Ready {},
            &cancel,
            Instant::now() + Duration::from_millis(20)
        ),
        Err(ControlError::Deadline)
    );
    assert_eq!(
        writer.send_lifecycle(
            ControlNotification::Ready {},
            &cancel,
            Instant::now() + Duration::from_secs(1)
        ),
        Err(ControlError::Deadline)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn legacy_writer_cannot_switch_version_or_deadline_mode() {
    use super::UnixControlWriter;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicBool;
    let cancel = AtomicBool::new(false);
    let (sender, _reader) = UnixStream::pair().unwrap();
    let mut legacy =
        UnixControlWriter::new(sender, [9; 32], Instant::now() + Duration::from_secs(1)).unwrap();
    assert_eq!(
        legacy.send_lifecycle(
            ControlNotification::Ready {},
            &cancel,
            Instant::now() + Duration::from_secs(1)
        ),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        legacy.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Protocol)
    );
    let (sender, _reader) = UnixStream::pair().unwrap();
    let mut lifecycle = UnixControlWriter::new_lifecycle(sender, [9; 32]).unwrap();
    assert_eq!(
        lifecycle.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Protocol)
    );
}
