//! Linux 原 socketpair 内核发送者、辅助 FD 和原预算验收，不替代 broker 的原 child wait。
use super::{
    ControlError, ControlFrame, ControlNotification, LinuxControlRead, LinuxControlReader,
};
use crate::recovery_slot::LinuxSupervisorPeer;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn peer(pid: libc::pid_t, uid: libc::uid_t, gid: libc::gid_t) -> LinuxSupervisorPeer {
    LinuxSupervisorPeer::from_host(pid, uid, gid).unwrap()
}
fn original_peer() -> LinuxSupervisorPeer {
    peer(
        unsafe { libc::getpid() },
        unsafe { libc::getuid() },
        unsafe { libc::getgid() },
    )
}
fn frame(sequence: u64, notification: ControlNotification) -> Vec<u8> {
    ControlFrame {
        version: 1,
        session: [7; 32],
        sequence,
        notification,
    }
    .encode()
    .unwrap()
}
fn endpoints() -> (UnixStream, UnixStream) {
    let (reader, writer) = UnixStream::pair().unwrap();
    (
        LinuxControlReader::prepare_receiver(reader).unwrap(),
        writer,
    )
}
fn reader(stream: UnixStream, peer: LinuxSupervisorPeer) -> LinuxControlReader {
    LinuxControlReader::new(
        stream,
        peer,
        [7; 32],
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap()
}

#[test]
fn actual_credentials_deliver_three_frames_and_confirmed_eof() {
    let (stream, mut writer) = endpoints();
    let mut reader = reader(stream, original_peer());
    for (sequence, notification) in [
        ControlNotification::Ready {},
        ControlNotification::ForegroundEnded {
            exit_code: 0,
            panicked: false,
        },
        ControlNotification::CleanupComplete {},
    ]
    .into_iter()
    .enumerate()
    {
        writer
            .write_all(&frame(sequence as u64, notification.clone()))
            .unwrap();
        assert_eq!(
            reader.poll(&AtomicBool::new(false)).unwrap(),
            LinuxControlRead::Frame(ControlFrame {
                version: 1,
                session: [7; 32],
                sequence: sequence as u64,
                notification,
            })
        );
    }
    drop(writer);
    assert_eq!(
        reader.poll(&AtomicBool::new(false)).unwrap(),
        LinuxControlRead::ConfirmedEof
    );
}

#[test]
fn empty_live_socket_returns_pending_without_waiting_for_original_deadline() {
    let (stream, _writer) = endpoints();
    assert_eq!(
        reader(stream, original_peer())
            .poll(&AtomicBool::new(false))
            .unwrap(),
        LinuxControlRead::Pending
    );
}

#[test]
fn unprepared_socket_is_refused() {
    let (stream, _writer) = UnixStream::pair().unwrap();
    assert!(matches!(
        LinuxControlReader::new(
            stream,
            original_peer(),
            [7; 32],
            Instant::now() + Duration::from_secs(5)
        ),
        Err(ControlError::Protocol)
    ));
}

#[test]
fn each_original_credential_component_is_required_and_failure_is_latched() {
    let pid = unsafe { libc::getpid() };
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    for expected in [
        peer(pid + 1, uid, gid),
        peer(pid, uid ^ 1, gid),
        peer(pid, uid, gid ^ 1),
    ] {
        let (stream, mut writer) = endpoints();
        let mut reader = reader(stream, expected);
        writer
            .write_all(&frame(0, ControlNotification::Ready {}))
            .unwrap();
        assert_eq!(
            reader.poll(&AtomicBool::new(false)),
            Err(ControlError::Protocol)
        );
        assert_eq!(
            reader.poll(&AtomicBool::new(false)),
            Err(ControlError::Protocol)
        );
    }
}

#[test]
fn socket_creator_credentials_do_not_authenticate_another_actual_writer() {
    let (stream, writer) = endpoints();
    let bytes = frame(0, ControlNotification::Ready {});
    let child = unsafe { libc::fork() };
    assert!(child >= 0);
    if child == 0 {
        // 多线程测试进程 fork 后只调用原 syscall，不分配、回调或运行 Rust 析构。
        let written =
            unsafe { libc::write(writer.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        unsafe {
            libc::_exit(if written == bytes.len() as isize {
                0
            } else {
                1
            })
        };
    }
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(child, &mut status, 0) }, child);
    assert_eq!(status, 0);
    let mut reader = reader(stream, original_peer());
    assert_eq!(
        reader.poll(&AtomicBool::new(false)),
        Err(ControlError::Protocol)
    );
}

#[test]
fn rights_and_truncated_rights_are_closed_before_rejection() {
    for count in [1, 80] {
        let (stream, writer) = endpoints();
        let source = tempfile::NamedTempFile::new().unwrap();
        let fd = source.as_file().as_raw_fd();
        let bytes = frame(0, ControlNotification::Ready {});
        let mut control = [0_usize; 128];
        let mut vector = libc::iovec {
            iov_base: bytes.as_ptr().cast_mut().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen =
            unsafe { libc::CMSG_SPACE((count * std::mem::size_of::<libc::c_int>()) as u32) }
                as usize;
        let header = unsafe { libc::CMSG_FIRSTHDR(&message) };
        unsafe {
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len =
                libc::CMSG_LEN((count * std::mem::size_of::<libc::c_int>()) as u32) as usize;
            let data = libc::CMSG_DATA(header).cast::<libc::c_int>();
            for index in 0..count {
                std::ptr::write_unaligned(data.add(index), fd);
            }
        }
        assert_eq!(
            unsafe { libc::sendmsg(writer.as_raw_fd(), &message, libc::MSG_NOSIGNAL) },
            bytes.len() as isize
        );
        let mut reader = reader(stream, original_peer());
        assert_eq!(
            reader.poll(&AtomicBool::new(false)),
            Err(ControlError::Protocol)
        );
        let matching_fds = std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                std::fs::read_link(entry.path()).is_ok_and(|path| path == source.path())
            })
            .count();
        assert_eq!(
            matching_fds, 1,
            "all delivered duplicates must close; source remains owned"
        );
    }
}

#[test]
fn incomplete_eof_is_not_confirmed() {
    let (stream, mut writer) = endpoints();
    let mut reader = reader(stream, original_peer());
    writer
        .write_all(&frame(0, ControlNotification::Ready {}))
        .unwrap();
    assert!(matches!(
        reader.poll(&AtomicBool::new(false)).unwrap(),
        LinuxControlRead::Frame(_)
    ));
    drop(writer);
    assert_eq!(
        reader.poll(&AtomicBool::new(false)),
        Err(ControlError::Unconfirmed)
    );
}

#[test]
fn cancellation_and_original_expiry_are_permanent() {
    let (stream, _writer) = endpoints();
    let mut reader = reader(stream, original_peer());
    assert_eq!(
        reader.poll(&AtomicBool::new(true)),
        Err(ControlError::Cancelled)
    );
    assert_eq!(
        reader.poll(&AtomicBool::new(false)),
        Err(ControlError::Cancelled)
    );
    let (stream, _writer) = endpoints();
    assert!(matches!(
        LinuxControlReader::new(stream, original_peer(), [7; 32], Instant::now()),
        Err(ControlError::Deadline)
    ));
}

#[test]
fn original_receiver_restores_close_on_exec_without_cloning_endpoint() {
    let (stream, _writer) = UnixStream::pair().unwrap();
    let original_fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(original_fd, libc::F_GETFD) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(original_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
        0
    );
    let stream = LinuxControlReader::prepare_receiver(stream).unwrap();
    assert_eq!(stream.as_raw_fd(), original_fd);
    assert_ne!(
        unsafe { libc::fcntl(original_fd, libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
}
