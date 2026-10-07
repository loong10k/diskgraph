//! Linux 原句柄和内核凭据的真实 socketpair 验证；来源：PF-06。不证明受信监督出生。
use super::{LinuxSupervisorMaterials, LinuxSupervisorPeer, LinuxSupervisorTrust};
use crate::recovery_control::ControlError;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixDatagram;
use std::os::unix::process::CommandExt;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn peer(pid_offset: i32) -> LinuxSupervisorPeer {
    LinuxSupervisorPeer::from_host(
        unsafe { libc::getpid() } + pid_offset,
        unsafe { libc::getuid() },
        unsafe { libc::getgid() },
    )
    .unwrap()
}

fn trust(root: File) -> LinuxSupervisorTrust {
    LinuxSupervisorTrust::from_host(
        root,
        File::open("/proc/thread-self/ns/user").unwrap(),
        File::open("/proc/thread-self/ns/mnt").unwrap(),
    )
}

#[test]
fn original_objects_arrive_with_close_on_exec_and_no_replay() {
    let cancel = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(5);
    let (sender, receiver) = UnixDatagram::pair().unwrap();
    let mut sender =
        LinuxSupervisorMaterials::new_sender(sender, peer(0), [7; 32], deadline).unwrap();
    let mut receiver = LinuxSupervisorMaterials::new(receiver, peer(0), [7; 32], deadline).unwrap();
    let source = trust(File::open("/").unwrap());
    sender.send(&source, &cancel).unwrap();
    let received = receiver.receive(&cancel).unwrap();
    for (original, received) in [
        (&source.root, &received.root),
        (&source.user_namespace, &received.user_namespace),
        (&source.mount_namespace, &received.mount_namespace),
    ] {
        let original_identity = original.metadata().unwrap();
        let received_identity = received.metadata().unwrap();
        assert_eq!(
            (original_identity.dev(), original_identity.ino()),
            (received_identity.dev(), received_identity.ino())
        );
        assert_ne!(
            unsafe { libc::fcntl(received.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
    }
    assert!(matches!(
        sender.send(&source, &cancel),
        Err(ControlError::Protocol)
    ));
    assert!(matches!(
        receiver.receive(&cancel),
        Err(ControlError::Protocol)
    ));
}

#[test]
fn wrong_session_or_kernel_peer_closes_all_received_root_references() {
    let fixture = tempfile::NamedTempFile::new().unwrap();
    let root = fixture.path().to_owned();
    let reference_count = || {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| std::fs::read_link(entry.path()).is_ok_and(|path| path == root))
            .count()
    };
    let source = trust(File::open(&root).unwrap());
    let before = reference_count();
    for wrong_peer in [false, true] {
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut sender =
            LinuxSupervisorMaterials::new_sender(sender, peer(0), [7; 32], deadline).unwrap();
        let mut receiver = LinuxSupervisorMaterials::new(
            receiver,
            peer(i32::from(wrong_peer)),
            if wrong_peer { [7; 32] } else { [8; 32] },
            deadline,
        )
        .unwrap();
        sender.send(&source, &AtomicBool::new(false)).unwrap();
        assert!(matches!(
            receiver.receive(&AtomicBool::new(false)),
            Err(ControlError::Protocol)
        ));
        assert!(matches!(
            receiver.receive(&AtomicBool::new(true)),
            Err(ControlError::Protocol)
        ));
        assert_eq!(
            reference_count(),
            before,
            "rejected packet leaked a unique root reference"
        );
    }
}

#[test]
fn original_expiry_and_cancel_refuse_wait_without_resending() {
    let (sender, receiver) = UnixDatagram::pair().unwrap();
    let mut sender =
        LinuxSupervisorMaterials::new_sender(sender, peer(0), [7; 32], Instant::now()).unwrap();
    let source = trust(File::open("/").unwrap());
    assert!(matches!(
        sender.send(&source, &AtomicBool::new(false)),
        Err(ControlError::Deadline)
    ));
    let mut receiver = LinuxSupervisorMaterials::new(
        receiver,
        peer(0),
        [7; 32],
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    assert!(matches!(
        receiver.receive(&AtomicBool::new(true)),
        Err(ControlError::Cancelled)
    ));
    assert!(matches!(
        receiver.receive(&AtomicBool::new(false)),
        Err(ControlError::Cancelled)
    ));
}

#[test]
fn extra_rights_and_truncated_packets_close_every_visible_reference() {
    let fixture = tempfile::NamedTempFile::new().unwrap();
    let reference_count = || {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                std::fs::read_link(entry.path()).is_ok_and(|path| path == fixture.path())
            })
            .count()
    };
    // 同一原文件重复传递形成不同接收 FD，能够检测拒绝分支的逐句柄释放。
    for (rights, payload) in [(4, 40), (253, 40), (3, 80)] {
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        let mut receiver = LinuxSupervisorMaterials::new(
            receiver,
            peer(0),
            [7; 32],
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let before = reference_count();
        let files = vec![fixture.as_raw_fd(); rights];
        let mut bytes = vec![7_u8; payload];
        bytes[..8].copy_from_slice(b"DGSM01A\n");
        let mut control = [0_usize; 160];
        let mut vector = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        let length = std::mem::size_of_val(files.as_slice());
        message.msg_controllen = unsafe { libc::CMSG_SPACE(length as u32) } as usize;
        assert!(message.msg_controllen <= std::mem::size_of_val(&control));
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(length as u32) as usize;
            std::ptr::copy_nonoverlapping(
                files.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(header),
                length,
            );
            assert_eq!(
                libc::sendmsg(sender.as_raw_fd(), &message, libc::MSG_NOSIGNAL),
                payload as isize
            );
        }
        assert!(matches!(
            receiver.receive(&AtomicBool::new(false)),
            Err(ControlError::Protocol)
        ));
        assert_eq!(
            reference_count(),
            before,
            "rights={rights}, payload={payload}"
        );
    }
}

#[test]
#[ignore = "only invoked by the original socketpair parent fixture"]
fn materials_child_fixture() {
    let mode = std::env::var("DISKGRAPH_MATERIALS_FIXTURE").unwrap();
    assert!(matches!(mode.as_str(), "receive" | "expired"));
    let socket = unsafe { UnixDatagram::from_raw_fd(3) };
    let parent = LinuxSupervisorPeer::from_host(
        unsafe { libc::getppid() },
        unsafe { libc::getuid() },
        unsafe { libc::getgid() },
    )
    .unwrap();
    let original: crate::native_deadline::ClockStamp =
        serde_json::from_str(&std::env::var("DISKGRAPH_MATERIALS_ORIGINAL_DEADLINE").unwrap())
            .unwrap();
    if mode == "expired" {
        std::thread::sleep(Duration::from_millis(40));
        assert!(matches!(
            original.adopt(Duration::from_secs(5)),
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::BudgetExceeded
            ))
        ));
        // 原期限已耗尽，不能创建材料接收能力；父侧不向该请求交付rights。
        return;
    }
    let deadline = original.adopt(Duration::from_secs(5)).unwrap();
    let mut receiver = LinuxSupervisorMaterials::new(socket, parent, [9; 32], deadline).unwrap();
    let received = receiver.receive(&AtomicBool::new(false)).unwrap();
    assert!(received.root.metadata().unwrap().is_dir());
    for (original, received) in [
        (File::open("/").unwrap(), &received.root),
        (
            File::open("/proc/thread-self/ns/user").unwrap(),
            &received.user_namespace,
        ),
        (
            File::open("/proc/thread-self/ns/mnt").unwrap(),
            &received.mount_namespace,
        ),
    ] {
        let expected = original.metadata().unwrap();
        let actual = received.metadata().unwrap();
        assert_eq!(
            (expected.dev(), expected.ino()),
            (actual.dev(), actual.ino())
        );
    }
}

#[test]
fn original_private_channel_delivers_to_a_real_child_process() {
    for expired in [false, true] {
        // 从端点准备和出生之前计时；子进程不能把排队/exec成本重新签发成完整5秒。
        let deadline = Instant::now() + Duration::from_secs(5);
        // 5秒是父侧有限观察预算；过期用例的原请求预算20ms，不得从观察预算续期。
        let original = crate::native_deadline::ClockStamp::capture(if expired {
            Instant::now() + Duration::from_millis(20)
        } else {
            deadline
        })
        .unwrap();
        let (sender, child_socket) = UnixDatagram::pair().unwrap();
        // 出生前使用真实库入口准备原接收端，不靠夹具自行设置选项。
        let child_socket = LinuxSupervisorMaterials::prepare_receiver(child_socket).unwrap();
        let fd = child_socket.as_raw_fd();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "recovery_slot::linux_supervisor_materials_tests::materials_child_fixture",
                "--ignored",
                "--test-threads=1",
            ])
            .env(
                "DISKGRAPH_MATERIALS_FIXTURE",
                if expired { "expired" } else { "receive" },
            )
            .env(
                "DISKGRAPH_MATERIALS_ORIGINAL_DEADLINE",
                serde_json::to_string(&original).unwrap(),
            );
        // 仅子进程 exec 前使用异步信号安全的原生调用；不在 Rust 测试线程中手工 fork。
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd, 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        drop(child_socket);
        let child_peer =
            LinuxSupervisorPeer::from_host(child.id() as i32, unsafe { libc::getuid() }, unsafe {
                libc::getgid()
            })
            .unwrap();
        let mut sender =
            LinuxSupervisorMaterials::new_sender(sender, child_peer, [9; 32], deadline).unwrap();
        let sent = if expired {
            Ok(())
        } else {
            sender.send(&trust(File::open("/").unwrap()), &AtomicBool::new(false))
        };
        let result = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        sent.unwrap();
        assert!(
            result.is_some_and(|status| status.success()),
            "child failed or exceeded original deadline"
        );
    }
}

#[test]
fn named_peers_and_wrong_direction_never_deliver_materials() {
    let directory = tempfile::tempdir().unwrap();
    let named = directory.path().join("named.sock");
    let _bound = UnixDatagram::bind(&named).unwrap();
    let client = UnixDatagram::unbound().unwrap();
    client.connect(&named).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    assert!(matches!(
        LinuxSupervisorMaterials::new_sender(client, peer(0), [7; 32], deadline),
        Err(ControlError::Protocol)
    ));
    let (sender, receiver) = UnixDatagram::pair().unwrap();
    let mut sender =
        LinuxSupervisorMaterials::new_sender(sender, peer(0), [7; 32], deadline).unwrap();
    let mut receiver = LinuxSupervisorMaterials::new(receiver, peer(0), [7; 32], deadline).unwrap();
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        sender.receive(&cancel),
        Err(ControlError::Protocol)
    ));
    assert!(matches!(
        receiver.send(&trust(File::open("/").unwrap()), &cancel),
        Err(ControlError::Protocol)
    ));
    // 错误方向永久锁存，不能用后续合法调用把错误恢复成一次交付。
    assert!(matches!(
        sender.send(&trust(File::open("/").unwrap()), &cancel),
        Err(ControlError::Protocol)
    ));
    assert!(matches!(
        receiver.receive(&cancel),
        Err(ControlError::Protocol)
    ));
}
