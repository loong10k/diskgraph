use super::UnixControlChannel;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

#[test]
fn foreground_input_fragments_are_exact_and_close_delivers_original_eof() {
    let (sender, mut receiver) = UnixStream::pair().unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut input = UnixControlChannel::from_stream(sender).unwrap();
    let bytes: Vec<_> = (0..6145).map(|i| (i % 251) as u8).collect();
    for chunk in bytes.chunks(4096) {
        input
            .write_chunk_until(
                chunk,
                Instant::now() + Duration::from_secs(1),
                &AtomicBool::new(false),
            )
            .unwrap();
    }
    input.close();
    let mut actual = Vec::new();
    receiver.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, bytes);
}

#[test]
fn expired_or_cancelled_input_sends_nothing_and_closes_without_resource_retirement() {
    for cancelled in [false, true] {
        let (sender, mut receiver) = UnixStream::pair().unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let (resource, _peer) = UnixStream::pair().unwrap();
        let mut input = UnixControlChannel::from_stream(sender).unwrap();
        let deadline = if cancelled {
            Instant::now() + Duration::from_secs(1)
        } else {
            Instant::now()
        };
        assert!(
            input
                .write_chunk_until(b"must not send", deadline, &AtomicBool::new(cancelled))
                .is_err()
        );
        assert!(input.start_write(b"cannot replay").is_err());
        let mut bytes = Vec::new();
        receiver.read_to_end(&mut bytes).unwrap();
        assert!(bytes.is_empty());
        assert!(resource.peer_addr().is_ok());
    }
}

#[test]
fn full_input_socket_keeps_original_deadline_and_discards_only_unsent_chunk() {
    let (mut sender, mut receiver) = UnixStream::pair().unwrap();
    sender.set_nonblocking(true).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    while sender.write(&[1; 4096]).is_ok() {}
    let mut input = UnixControlChannel::from_stream(sender).unwrap();
    let started = Instant::now();
    assert!(
        input
            .write_chunk_until(
                &[2; 4096],
                started + Duration::from_millis(20),
                &AtomicBool::new(false)
            )
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        input.poll_write().unwrap(),
        super::super::ControlWriteStatus::Closed
    );
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    assert!(!bytes.is_empty());
    assert!(bytes.iter().all(|byte| *byte == 1));
}
