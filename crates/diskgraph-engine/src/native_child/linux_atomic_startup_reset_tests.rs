//! 真实 Linux SOCK_SEQPACKET 的待处理 reset 不能遮蔽已排队的子初始化错误。
use super::ChildSpawnError;
use super::linux_atomic_handshake::LinuxAtomicHandshake;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

#[test]
fn queued_child_failure_survives_reset_from_unread_init() {
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
    // 实际发送未被子端消费的 Init，以及真实固定格式 Error。关闭子端产生内核 reset。
    send(parent.as_raw_fd(), [3, 0, 0]);
    send(child.as_raw_fd(), [2, 2, libc::EACCES as u32]);
    drop(child);
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
