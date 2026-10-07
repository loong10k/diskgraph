#![cfg(any(target_os = "linux", target_os = "macos"))]
use diskgraph_engine::recovery_control::{
    ControlError, ControlFrame, ControlNotification, UnixControlReader,
};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
fn frame(sequence: u64, notification: ControlNotification) -> Vec<u8> {
    ControlFrame {
        version: 1,
        session: [1; 32],
        sequence,
        notification,
    }
    .encode()
    .unwrap()
}
#[test]
fn actual_socket_preserves_frames_and_confirmed_protocol_eof() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let expected = [
        ControlNotification::Ready {},
        ControlNotification::ForegroundEnded {
            exit_code: 42,
            panicked: true,
        },
        ControlNotification::CleanupComplete {},
    ];
    for (sequence, notification) in expected.iter().enumerate() {
        peer.write_all(&frame(sequence as u64, notification.clone()))
            .unwrap();
    }
    drop(peer);
    let mut reader =
        UnixControlReader::new(stream, [1; 32], Instant::now() + Duration::from_secs(2)).unwrap();
    let cancel = AtomicBool::new(false);
    for notification in expected {
        assert_eq!(
            reader.receive(&cancel).unwrap().unwrap().notification,
            notification
        );
    }
    assert_eq!(reader.receive(&cancel).unwrap(), None);
}
#[test]
fn actual_partial_eof_and_foreground_only_remain_unconfirmed() {
    for complete_foreground in [false, true] {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(&frame(0, ControlNotification::Ready {}))
            .unwrap();
        let foreground = frame(
            1,
            ControlNotification::ForegroundEnded {
                exit_code: 0,
                panicked: false,
            },
        );
        peer.write_all(if complete_foreground {
            &foreground
        } else {
            &foreground[..5]
        })
        .unwrap();
        drop(peer);
        let mut reader =
            UnixControlReader::new(stream, [1; 32], Instant::now() + Duration::from_secs(2))
                .unwrap();
        let cancel = AtomicBool::new(false);
        assert!(reader.receive(&cancel).unwrap().is_some());
        if complete_foreground {
            assert!(reader.receive(&cancel).unwrap().is_some());
        }
        assert_eq!(reader.receive(&cancel), Err(ControlError::Unconfirmed));
        assert_eq!(reader.receive(&cancel), Err(ControlError::Unconfirmed));
    }
}
#[test]
fn waiting_original_socket_stops_when_cancelled_and_stays_stopped() {
    let (stream, peer) = UnixStream::pair().unwrap();
    let mut reader =
        UnixControlReader::new(stream, [1; 32], Instant::now() + Duration::from_secs(2)).unwrap();
    let cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(20));
            cancel.store(true, Ordering::Release);
        });
        assert_eq!(reader.receive(&cancel), Err(ControlError::Cancelled));
    });
    cancel.store(false, Ordering::Release);
    assert_eq!(reader.receive(&cancel), Err(ControlError::Cancelled));
    drop(peer);
}
#[test]
fn waiting_actual_socket_cannot_refresh_original_deadline() {
    let (stream, peer) = UnixStream::pair().unwrap();
    let mut reader =
        UnixControlReader::new(stream, [1; 32], Instant::now() + Duration::from_millis(20))
            .unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(reader.receive(&cancel), Err(ControlError::Deadline));
    assert_eq!(reader.receive(&cancel), Err(ControlError::Deadline));
    drop(peer);
}
#[test]
fn wrong_session_on_actual_socket_is_sticky() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    peer.write_all(&frame(0, ControlNotification::Ready {}))
        .unwrap();
    let mut reader =
        UnixControlReader::new(stream, [2; 32], Instant::now() + Duration::from_secs(2)).unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(reader.receive(&cancel), Err(ControlError::Protocol));
    assert_eq!(reader.receive(&cancel), Err(ControlError::Protocol));
}

#[test]
fn later_frames_cannot_escape_after_original_read_deadline() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let mut bytes = frame(0, ControlNotification::Ready {});
    bytes.extend(frame(
        1,
        ControlNotification::ForegroundEnded {
            exit_code: 0,
            panicked: false,
        },
    ));
    peer.write_all(&bytes).unwrap();
    let deadline = Instant::now() + Duration::from_millis(200);
    let mut reader = UnixControlReader::new(stream, [1; 32], deadline).unwrap();
    let cancel = AtomicBool::new(false);
    assert!(reader.receive(&cancel).unwrap().is_some());
    // 数据已由真实对端写入，后续交付仍受同一期限约束，不能为待交付数据续期。
    std::thread::sleep(
        deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
    );
    assert_eq!(reader.receive(&cancel), Err(ControlError::Deadline));
    drop(peer);
}
