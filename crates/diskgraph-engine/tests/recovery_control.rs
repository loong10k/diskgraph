use diskgraph_engine::recovery_control::{
    ControlError, ControlFrame, ControlNotification as Notice, ControlReceiver,
};
use std::time::{Duration, Instant};
fn frame(sequence: u64, notification: Notice) -> ControlFrame {
    ControlFrame {
        version: 1,
        session: [7; 32],
        sequence,
        notification,
    }
}
fn receiver() -> ControlReceiver {
    ControlReceiver::new([7; 32], Instant::now() + Duration::from_secs(2))
}
#[test]
fn split_frames_preserve_foreground_and_cleanup_as_different_events() {
    let events = [
        Notice::Ready {},
        Notice::ForegroundEnded {
            exit_code: -1234567,
            panicked: true,
        },
        Notice::CleanupPending {},
        Notice::CleanupComplete {},
    ];
    let bytes: Vec<u8> = events
        .iter()
        .enumerate()
        .flat_map(|(i, n)| frame(i as u64, n.clone()).encode().unwrap())
        .collect();
    let mut r = receiver();
    let mut got = Vec::new();
    for byte in bytes {
        got.extend(r.push(&[byte]).unwrap());
    }
    assert_eq!(
        got.iter()
            .map(|f| f.notification.clone())
            .collect::<Vec<_>>(),
        events
    );
    r.end_of_stream().unwrap();
    assert_eq!(
        r.push(&frame(4, Notice::CleanupComplete {}).encode().unwrap()),
        Err(ControlError::Protocol)
    );
}
#[test]
fn rejects_wrong_session_version_sequence_and_premature_cleanup() {
    let mut variants = vec![
        frame(0, Notice::CleanupComplete {}),
        frame(1, Notice::Ready {}),
    ];
    let mut wrong = frame(0, Notice::Ready {});
    wrong.session = [8; 32];
    variants.push(wrong);
    let mut old = frame(0, Notice::Ready {});
    old.version = 0;
    variants.push(old);
    for f in variants {
        let mut r = receiver();
        assert_eq!(r.push(&f.encode().unwrap()), Err(ControlError::Protocol));
        assert_eq!(
            r.push(&frame(0, Notice::Ready {}).encode().unwrap()),
            Err(ControlError::Protocol)
        );
    }
}
#[test]
fn partial_eof_and_foreground_eof_do_not_confirm_cleanup() {
    let mut r = receiver();
    r.push(&[0, 0]).unwrap();
    assert_eq!(r.end_of_stream(), Err(ControlError::Unconfirmed));
    let mut r = receiver();
    r.push(&frame(0, Notice::Ready {}).encode().unwrap())
        .unwrap();
    r.push(
        &frame(
            1,
            Notice::ForegroundEnded {
                exit_code: 0,
                panicked: false,
            },
        )
        .encode()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(r.end_of_stream(), Err(ControlError::Unconfirmed));
}
#[test]
fn oversized_header_is_rejected_before_body_allocation_and_failure_is_sticky() {
    let mut r = receiver();
    assert_eq!(r.push(&4097u32.to_be_bytes()), Err(ControlError::Budget));
    assert_eq!(r.push(&[]), Err(ControlError::Budget));
    let mut r = receiver();
    assert_eq!(r.push(&vec![0; 65537]), Err(ControlError::Budget));
}
#[test]
fn original_deadline_is_never_refreshed() {
    let mut r = ControlReceiver::new([7; 32], Instant::now() - Duration::from_millis(1));
    assert_eq!(
        r.push(&frame(0, Notice::Ready {}).encode().unwrap()),
        Err(ControlError::Deadline)
    );
}
#[test]
fn repeated_pending_notifications_cannot_exceed_original_frame_count() {
    let mut r = receiver();
    r.push(&frame(0, Notice::Ready {}).encode().unwrap())
        .unwrap();
    r.push(
        &frame(
            1,
            Notice::ForegroundEnded {
                exit_code: 0,
                panicked: false,
            },
        )
        .encode()
        .unwrap(),
    )
    .unwrap();
    for i in 2..64 {
        r.push(&frame(i, Notice::CleanupPending {}).encode().unwrap())
            .unwrap();
    }
    assert_eq!(
        r.push(&frame(64, Notice::CleanupPending {}).encode().unwrap()),
        Err(ControlError::Budget)
    );
}

#[test]
fn malformed_duplicate_or_unknown_fields_are_rejected_without_reopening_receiver() {
    for body in [
        br#"{"version":1,"version":1}"#.as_slice(),
        br#"{"version":1,"session":[],"sequence":0,"notification":{"kind":"ready"}}"#.as_slice(),
        br#"{"version":1,"session":[7,7],"sequence":0,"notification":{"kind":"ready"},"arbitrary_command":"run"}"#.as_slice(),
    ] {
        let mut bytes=(body.len() as u32).to_be_bytes().to_vec();bytes.extend_from_slice(body);
        let mut r=receiver();assert_eq!(r.push(&bytes),Err(ControlError::Protocol));
        assert_eq!(r.end_of_stream(),Err(ControlError::Protocol));
    }
    assert_eq!(receiver().push(&[0; 4]), Err(ControlError::Protocol));
}

#[test]
fn a_fragment_cannot_renew_the_original_deadline() {
    let mut r = ControlReceiver::new([7; 32], Instant::now() + Duration::from_millis(30));
    let bytes = frame(0, Notice::Ready {}).encode().unwrap();
    r.push(&bytes[..2]).unwrap();
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(r.push(&bytes[2..]), Err(ControlError::Deadline));
    assert_eq!(r.end_of_stream(), Err(ControlError::Deadline));
}

#[cfg(unix)]
#[test]
fn actual_private_socket_eof_after_foreground_is_unconfirmed() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let (mut sender, mut socket) = UnixStream::pair().unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut r = receiver();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            for f in [
                frame(0, Notice::Ready {}),
                frame(
                    1,
                    Notice::ForegroundEnded {
                        exit_code: 9,
                        panicked: false,
                    },
                ),
            ] {
                for chunk in f.encode().unwrap().chunks(7) {
                    sender.write_all(chunk).unwrap();
                }
            }
            // 真实关闭私有 socket，不发送 CleanupComplete；不能用 EOF 推定回收。
        });
        let mut events = Vec::new();
        let mut buffer = [0u8; 3];
        loop {
            let n = socket.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            events.extend(r.push(&buffer[..n]).unwrap());
        }
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1].notification,
            Notice::ForegroundEnded {
                exit_code: 9,
                panicked: false
            }
        );
        assert_eq!(r.end_of_stream(), Err(ControlError::Unconfirmed));
    });
}

#[test]
fn otherwise_valid_frames_reject_unknown_top_level_and_notification_fields() {
    for nested in [false, true] {
        let mut value = serde_json::to_value(frame(0, Notice::Ready {})).unwrap();
        if nested {
            value["notification"]["unexpected"] = serde_json::json!(true);
        } else {
            value["unexpected"] = serde_json::json!(true);
        }
        let body = serde_json::to_vec(&value).unwrap();
        let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&body);
        assert_eq!(receiver().push(&bytes), Err(ControlError::Protocol));
    }
}
