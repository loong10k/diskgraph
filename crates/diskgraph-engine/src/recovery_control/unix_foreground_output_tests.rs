use super::UnixForegroundOutput;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

#[test]
fn public_eof_does_not_wait_for_an_independent_original_resource() {
    let (sender, mut receiver) = UnixStream::pair().unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let (original_resource, _peer) = UnixStream::pair().unwrap();
    let mut output = UnixForegroundOutput::new(sender).unwrap();
    output
        .write_chunk_until(
            b"original reply\n",
            Instant::now() + Duration::from_secs(1),
            &AtomicBool::new(false),
        )
        .unwrap();
    output.finish().unwrap();
    output.finish().unwrap();
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"original reply\n");
    assert!(original_resource.peer_addr().is_ok());
}

#[test]
fn cancelled_or_expired_delivery_is_latched_and_cannot_be_replayed() {
    for cancelled in [false, true] {
        let (sender, mut receiver) = UnixStream::pair().unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut output = UnixForegroundOutput::new(sender).unwrap();
        let deadline = if cancelled {
            Instant::now() + Duration::from_secs(1)
        } else {
            Instant::now()
        };
        assert!(
            output
                .write_chunk_until(b"reply", deadline, &AtomicBool::new(cancelled))
                .is_err()
        );
        assert!(
            output
                .write_chunk_until(
                    b"reply",
                    Instant::now() + Duration::from_secs(1),
                    &AtomicBool::new(false)
                )
                .is_err()
        );
        let mut bytes = Vec::new();
        receiver.read_to_end(&mut bytes).unwrap();
        assert!(bytes.is_empty());
    }
}

#[test]
fn a_full_socket_respects_the_original_deadline_and_broken_peer_is_sigpipe_safe() {
    let (mut sender, _receiver) = UnixStream::pair().unwrap();
    sender.set_nonblocking(true).unwrap();
    while sender.write(&[1; 4096]).is_ok() {}
    let mut output = UnixForegroundOutput::new(sender).unwrap();
    let started = Instant::now();
    assert_eq!(
        output
            .write_chunk_until(
                &[2; 4096],
                started + Duration::from_millis(20),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    let (sender, receiver) = UnixStream::pair().unwrap();
    let mut output = UnixForegroundOutput::new(sender).unwrap();
    drop(receiver);
    assert!(
        output
            .write_chunk_until(
                b"reply",
                Instant::now() + Duration::from_secs(1),
                &AtomicBool::new(false)
            )
            .is_err()
    );
}
