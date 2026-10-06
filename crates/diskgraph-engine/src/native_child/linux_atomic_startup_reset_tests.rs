//! 真实 Linux SOCK_SEQPACKET 的待处理 reset 不能遮蔽已排队的子初始化错误。
use super::ChildSpawnError;
use super::linux_atomic_handshake::LinuxAtomicHandshake;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

#[test]
fn queued_child_failure_survives_reset_from_unread_init() {
    let parent = queued(Some([2, 2, libc::EACCES as u32]));
    let deadline = Instant::now() + Duration::from_secs(5);
    let received =
        LinuxAtomicHandshake::receive(parent.as_raw_fd(), deadline, &mut || Ok::<_, ()>(()));
    let error = match received {
        Ok(Some(Some(message))) => message
            .verify()
            .expect_err("failure packet must not become Ready"),
        Err(ChildSpawnError::Operation(error)) => error,
        _ => panic!("queued original child failure was lost"),
    };
    assert_eq!(
        error.native_io_error().unwrap().raw_os_error(),
        Some(libc::EACCES),
        "pending socket reset must not hide the queued close_range failure"
    );
}

#[test]
fn reset_does_not_promote_ready_invalid_packet_or_eof_to_success() {
    for packet in [Some([1, 0, 0]), Some([2, 99, 13]), None] {
        let parent = queued(packet);
        let result = LinuxAtomicHandshake::receive(
            parent.as_raw_fd(),
            Instant::now() + Duration::from_secs(5),
            &mut || Ok::<_, ()>(()),
        );
        match result {
            Err(ChildSpawnError::Operation(error)) => assert_eq!(
                error.native_io_error().unwrap().raw_os_error(),
                Some(libc::ECONNRESET)
            ),
            _ => panic!("socket reset cannot become startup success"),
        }
    }
}

#[test]
fn reset_recovery_rechecks_original_nonclone_authorization() {
    let parent = queued(Some([2, 2, libc::EACCES as u32]));
    let mut calls = 0;
    let result = LinuxAtomicHandshake::receive(
        parent.as_raw_fd(),
        Instant::now() + Duration::from_secs(5),
        &mut || {
            calls += 1;
            if calls == 2 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "original revoked request",
                ))
            } else {
                Ok(())
            }
        },
    );
    assert_eq!(
        calls, 2,
        "recovery must borrow the original checkpoint again"
    );
    match result {
        Err(ChildSpawnError::Checkpoint { primary, .. }) => {
            assert_eq!(primary.to_string(), "original revoked request")
        }
        _ => panic!("original authorization error must survive recovery"),
    }
}

fn queued(packet: Option<[u32; 3]>) -> OwnedFd {
    let mut fds = [-1; 2];
    assert_eq!(
        unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                0,
                fds.as_mut_ptr(),
            )
        },
        0
    );
    let parent = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let child = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    // 未消费的Init导致实际内核reset；接收队列仍保留子端已发送的固定消息。
    send(parent.as_raw_fd(), [3, 0, 0]);
    if let Some(packet) = packet {
        send(child.as_raw_fd(), packet);
    }
    drop(child);
    parent
}

fn send(fd: i32, packet: [u32; 3]) {
    assert_eq!(
        unsafe {
            libc::send(
                fd,
                packet.as_ptr().cast(),
                std::mem::size_of_val(&packet),
                libc::MSG_NOSIGNAL,
            )
        },
        12
    );
}
