use super::{ControlError, LinuxExecutorMaterials};
use crate::recovery_slot::LinuxSupervisorPeer;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn identity() -> LinuxSupervisorPeer {
    LinuxSupervisorPeer::from_host(
        unsafe { libc::getpid() },
        unsafe { libc::getuid() },
        unsafe { libc::getgid() },
    )
    .unwrap()
}
fn endpoints() -> (UnixDatagram, UnixDatagram) {
    let (rx, tx) = UnixDatagram::pair().unwrap();
    (LinuxExecutorMaterials::prepare_receiver(rx).unwrap(), tx)
}
fn receiver(rx: UnixDatagram, session: [u8; 32]) -> LinuxExecutorMaterials {
    LinuxExecutorMaterials::receiver(
        rx,
        identity(),
        session,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap()
}

#[test]
fn actual_kernel_credentials_authenticate_once_without_any_rights() {
    let (rx, tx) = endpoints();
    let mut rx = receiver(rx, [7; 32]);
    let mut tx =
        LinuxExecutorMaterials::sender(tx, [7; 32], Instant::now() + Duration::from_secs(5))
            .unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(rx.poll_receive(&cancel), Ok(false));
    assert_eq!(tx.poll_send(&cancel), Ok(true));
    assert_eq!(rx.poll_receive(&cancel), Ok(true));
    assert_eq!(rx.poll_receive(&cancel), Err(ControlError::Protocol));
    assert_eq!(tx.poll_send(&cancel), Err(ControlError::Protocol));
}

#[test]
fn wrong_actual_sender_identity_is_rejected_and_failure_is_latched() {
    let (rx, tx) = endpoints();
    let wrong = LinuxSupervisorPeer::from_host(
        unsafe { libc::getpid() } + 1,
        unsafe { libc::getuid() },
        unsafe { libc::getgid() },
    )
    .unwrap();
    let mut rx = LinuxExecutorMaterials::receiver(
        rx,
        wrong,
        [7; 32],
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    let mut tx =
        LinuxExecutorMaterials::sender(tx, [7; 32], Instant::now() + Duration::from_secs(5))
            .unwrap();
    tx.poll_send(&AtomicBool::new(false)).unwrap();
    assert_eq!(
        rx.poll_receive(&AtomicBool::new(false)),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        rx.poll_receive(&AtomicBool::new(false)),
        Err(ControlError::Protocol)
    );
}

#[test]
fn wrong_session_and_truncated_payload_are_rejected() {
    for bytes in [
        b"DGEX01A\n"
            .iter()
            .copied()
            .chain([8; 32])
            .collect::<Vec<_>>(),
        vec![9; 41],
        vec![9; 39],
    ] {
        let (rx, tx) = endpoints();
        let mut rx = receiver(rx, [7; 32]);
        tx.send(&bytes).unwrap();
        assert_eq!(
            rx.poll_receive(&AtomicBool::new(false)),
            Err(ControlError::Protocol)
        );
    }
}

#[test]
fn receiver_must_be_prepared_before_sender_can_run() {
    let (rx, _tx) = UnixDatagram::pair().unwrap();
    assert!(
        LinuxExecutorMaterials::receiver(
            rx,
            identity(),
            [7; 32],
            Instant::now() + Duration::from_secs(5)
        )
        .is_err()
    );
}

#[test]
fn unexpected_rights_are_closed_before_protocol_rejection() {
    let (rx, tx) = endpoints();
    let mut rx = receiver(rx, [7; 32]);
    let (mut observer, delivered) = UnixStream::pair().unwrap();
    observer.set_nonblocking(true).unwrap();
    let mut bytes = b"DGEX01A\n"
        .iter()
        .copied()
        .chain([7; 32])
        .collect::<Vec<_>>();
    let mut vector = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut control = [0_usize; 16];
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen =
        unsafe { libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>() as u32) } as usize;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as u32) as usize;
        std::ptr::write_unaligned(
            libc::CMSG_DATA(header).cast::<libc::c_int>(),
            delivered.as_raw_fd(),
        );
        assert_eq!(
            libc::sendmsg(tx.as_raw_fd(), &message, libc::MSG_NOSIGNAL),
            40
        );
    }
    drop(delivered);
    let mut scratch = [0; 1];
    assert_eq!(
        observer.read(&mut scratch).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        rx.poll_receive(&AtomicBool::new(false)),
        Err(ControlError::Protocol)
    );
    assert_eq!(observer.read(&mut scratch).unwrap(), 0);
}

#[test]
fn expired_and_cancelled_authentication_never_renews_budget() {
    let (rx, tx) = endpoints();
    assert!(
        LinuxExecutorMaterials::receiver(
            rx,
            identity(),
            [7; 32],
            Instant::now() - Duration::from_secs(1)
        )
        .is_err()
    );
    let mut tx =
        LinuxExecutorMaterials::sender(tx, [7; 32], Instant::now() + Duration::from_secs(5))
            .unwrap();
    assert_eq!(
        tx.poll_send(&AtomicBool::new(true)),
        Err(ControlError::Cancelled)
    );
    assert_eq!(
        tx.poll_send(&AtomicBool::new(false)),
        Err(ControlError::Cancelled)
    );
}
