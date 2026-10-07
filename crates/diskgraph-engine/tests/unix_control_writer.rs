#![cfg(any(target_os = "linux", target_os = "macos"))]
use diskgraph_engine::recovery_control::{
    ControlError, ControlNotification, ControlReceiver, UnixControlWriter,
};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
#[test]
fn actual_full_socket_buffer_stops_under_original_deadline() {
    let (mut original, mut peer) = UnixStream::pair().unwrap();
    original.set_nonblocking(true).unwrap();
    let block = [0u8; 1];
    loop {
        match original.write(&block) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => panic!("fill original socket: {e}"),
        }
    }
    original.set_nonblocking(false).unwrap();
    let until = Instant::now() + Duration::from_millis(40);
    let mut writer = UnixControlWriter::new(original, [3; 32], until).unwrap();
    std::thread::scope(|scope| {
        // 后备解除真实背压，避免错误阻塞原型使本机测试永远挂起；不作为写入成功证据。
        scope.spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            peer.set_nonblocking(true).unwrap();
            let mut b = [0; 8192];
            loop {
                match peer.read(&mut b) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("drain peer: {e}"),
                }
            }
        });
        let result = writer.send(ControlNotification::Ready {}, &AtomicBool::new(false));
        assert_eq!(
            result,
            Err(ControlError::Deadline),
            "real full socket must not write after original deadline"
        );
    });
}
#[test]
fn actual_socket_delivers_original_frames() {
    let (original, mut peer) = UnixStream::pair().unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let mut writer = UnixControlWriter::new(original, [8; 32], until).unwrap();
    let cancel = AtomicBool::new(false);
    writer.send(ControlNotification::Ready {}, &cancel).unwrap();
    writer
        .send(
            ControlNotification::ForegroundEnded {
                exit_code: 42,
                panicked: false,
            },
            &cancel,
        )
        .unwrap();
    writer
        .send(ControlNotification::CleanupComplete {}, &cancel)
        .unwrap();
    drop(writer);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).unwrap();
    let mut receiver = ControlReceiver::new([8; 32], until);
    let frames = receiver.push(&bytes).unwrap();
    assert_eq!(frames.len(), 3);
    receiver.end_of_stream().unwrap();
}
#[test]
fn cancellation_is_sticky() {
    let (original, peer) = UnixStream::pair().unwrap();
    let mut writer =
        UnixControlWriter::new(original, [1; 32], Instant::now() + Duration::from_secs(2)).unwrap();
    let cancel = AtomicBool::new(true);
    assert_eq!(
        writer.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Cancelled)
    );
    cancel.store(false, std::sync::atomic::Ordering::Release);
    assert_eq!(
        writer.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Cancelled)
    );
    drop(peer);
}

#[test]
fn actual_peer_close_is_sticky_and_does_not_raise_sigpipe() {
    let (original, peer) = UnixStream::pair().unwrap();
    let mut writer =
        UnixControlWriter::new(original, [1; 32], Instant::now() + Duration::from_secs(2)).unwrap();
    drop(peer);
    let cancel = AtomicBool::new(false);
    assert_eq!(
        writer.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Unconfirmed)
    );
    assert_eq!(
        writer.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Unconfirmed)
    );
}
#[test]
fn invalid_order_does_not_write_and_remains_rejected() {
    let (original, mut peer) = UnixStream::pair().unwrap();
    let mut writer =
        UnixControlWriter::new(original, [1; 32], Instant::now() + Duration::from_secs(2)).unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        writer.send(ControlNotification::CleanupComplete {}, &cancel),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        writer.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Protocol)
    );
    peer.set_nonblocking(true).unwrap();
    assert_eq!(
        peer.read(&mut [0; 32]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
#[test]
fn same_original_frame_budget_refuses_further_writes() {
    let (original, peer) = UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    // 累计帧预算独立于内核缓冲容量；真实对端持续读取并核对全部已交付帧。
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        peer.take(65537).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() <= 65536);
        bytes
    });
    let mut writer =
        UnixControlWriter::new(original, [1; 32], Instant::now() + Duration::from_secs(2)).unwrap();
    let cancel = AtomicBool::new(false);
    writer.send(ControlNotification::Ready {}, &cancel).unwrap();
    writer
        .send(
            ControlNotification::ForegroundEnded {
                exit_code: 0,
                panicked: false,
            },
            &cancel,
        )
        .unwrap();
    for _ in 0..62 {
        writer
            .send(ControlNotification::CleanupPending {}, &cancel)
            .unwrap();
    }
    assert_eq!(
        writer.send(ControlNotification::CleanupPending {}, &cancel),
        Err(ControlError::Budget)
    );
    drop(writer);
    let bytes = reader.join().unwrap();
    let mut receiver = ControlReceiver::new([1; 32], Instant::now() + Duration::from_secs(2));
    assert_eq!(receiver.push(&bytes).unwrap().len(), 64);
    assert_eq!(receiver.end_of_stream(), Err(ControlError::Unconfirmed));
}

#[test]
fn cancellation_during_actual_backpressure_stops_original_writer() {
    let (mut original, peer) = UnixStream::pair().unwrap();
    original.set_nonblocking(true).unwrap();
    // 单字节填满真实 socket，避免大块写失败后仍有小帧可用空间。
    loop {
        match original.write(&[0]) {
            Ok(1) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            result => panic!("unexpected fill result: {result:?}"),
        }
    }
    let until = Instant::now() + Duration::from_secs(2);
    let mut writer = UnixControlWriter::new(original, [7; 32], until).unwrap();
    let cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(20));
            cancel.store(true, std::sync::atomic::Ordering::Release);
        });
        assert_eq!(
            writer.send(ControlNotification::Ready {}, &cancel),
            Err(ControlError::Cancelled)
        );
    });
    cancel.store(false, std::sync::atomic::Ordering::Release);
    assert_eq!(
        writer.send(ControlNotification::Ready {}, &cancel),
        Err(ControlError::Cancelled)
    );
    drop(peer);
}
