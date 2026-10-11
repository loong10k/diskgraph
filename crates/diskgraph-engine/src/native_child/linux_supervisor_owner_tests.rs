//! 新监督出生入口与现有原 pidfd owner 的真实桥接资格；不代替实际产品 broker。

use super::linux_atomic_child::LinuxAtomicChild;
use super::linux_atomic_exit::LinuxAtomicExit;
use super::linux_atomic_handshake::LinuxAtomicHandshake;
use super::linux_atomic_pipes::LinuxAtomicPipes;
use super::linux_atomic_signal_mask::LinuxAtomicSignalMask;
use super::unix_child_setup::UnixChildSetup;
use std::ffi::{CString, c_char};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

/// 出生前固定七描述符材料；来源：linux_supervisor_spawn.h 的 native64 C ABI。
#[repr(C)]
struct SupervisorRequest {
    fds: [i32; 7],
    uid: u32,
    gid: u32,
    argv: *const *const c_char,
    environment: *const *const c_char,
}

unsafe extern "C" {
    fn dg_linux_supervisor_spawn(
        request: *const SupervisorRequest,
        pidfd: *mut i32,
        old_mask: *mut u64,
        mask_active: *mut i32,
    ) -> libc::c_long;
}

fn duplicate(fd: i32) -> OwnedFd {
    let result = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
    assert!(result >= 10, "duplicate original descriptor");
    unsafe { OwnedFd::from_raw_fd(result) }
}

fn birth() -> (LinuxAtomicChild, OwnedFd) {
    assert_eq!(
        unsafe { libc::getuid() },
        0,
        "isolated root qualification only"
    );
    UnixChildSetup::reject_auto_reap().unwrap();
    let path = std::env::var_os("DG_LINUX_SUPERVISOR_ROLE")
        .expect("explicit compiled ELF fixture required");
    let image = File::open(path).unwrap();
    let pipes = LinuxAtomicPipes::new(&image).unwrap();
    let (materials, child_materials) = UnixDatagram::pair().unwrap();
    let (control, child_control) = UnixStream::pair().unwrap();
    let extra = [
        duplicate(child_materials.as_raw_fd()),
        duplicate(child_control.as_raw_fd()),
    ];
    let base = pipes.child_fds();
    let argument = CString::new("fixed-supervisor-role").unwrap();
    let argv = [argument.as_ptr(), std::ptr::null()];
    let environment = [std::ptr::null()];
    let request = SupervisorRequest {
        fds: [
            base[0],
            base[1],
            base[2],
            base[3],
            base[4],
            extra[0].as_raw_fd(),
            extra[1].as_raw_fd(),
        ],
        uid: 23111,
        gid: 23111,
        argv: argv.as_ptr(),
        environment: environment.as_ptr(),
    };
    // 唯一处置槽在内核出生前分配；到原 fd 接管之间没有断言、回调或可失败操作。
    let mut slot = Box::<LinuxAtomicChild>::new_uninit();
    let mut mask = LinuxAtomicSignalMask::new();
    let mut pidfd = -1;
    let pid = unsafe {
        dg_linux_supervisor_spawn(&request, &mut pidfd, &mut mask.original, &mut mask.active)
    };
    if pid < 0 {
        mask.restore().unwrap();
        panic!("original supervisor birth errno: {}", -pid);
    }
    let fd = unsafe { OwnedFd::from_raw_fd(pidfd) };
    slot.write(LinuxAtomicChild::adopt(
        LinuxAtomicExit::adopt(fd, pid as i32),
        pipes,
    ));
    let mut child = unsafe { slot.assume_init() };
    // owner 已存在，才允许恢复 mask、配置 I/O 和建立测试观察句柄。
    mask.restore().unwrap();
    drop((extra, child_materials, child_control));
    child.configure().unwrap();
    let witness = duplicate(child.pidfd_for_test());
    drop((materials, control));
    (*child, witness)
}

fn assert_original_wait_consumed(witness: &OwnedFd) {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PIDFD,
            witness.as_raw_fd() as u32,
            &mut info,
            libc::WEXITED | libc::WNOHANG,
        )
    };
    assert_eq!(result, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn supervisor_birth_reuses_original_owner_for_normal_retirement() {
    let (mut child, witness) = birth();
    let deadline = Instant::now() + Duration::from_secs(5);
    LinuxAtomicHandshake::run(&mut child, deadline, &mut || Ok::<(), ()>(())).unwrap();
    child.request_control_close().unwrap();
    let mut output = Vec::new();
    loop {
        assert!(Instant::now() < deadline, "original retirement deadline");
        if let Some(bytes) = child.read_stdout().unwrap() {
            output.extend_from_slice(bytes);
            assert!(output.len() <= 4096, "fixed fixture output budget");
        }
        child.read_stderr().unwrap();
        if child.poll_normal_exit(&mut || Ok::<(), ()>(())).unwrap() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let report: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(child.exit_code(), Some(0));
    assert!(child.reaped());
    assert_original_wait_consumed(&witness);
}

#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn supervisor_birth_expired_cleanup_retains_owner_before_init() {
    let (mut child, witness) = birth();
    assert!(!child.poll_cleanup(Instant::now()).unwrap());
    assert!(!child.reaped());
    assert!(!child.poll().unwrap());
    child.cleanup().unwrap();
    assert!(child.reaped());
    assert_original_wait_consumed(&witness);
}

#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn supervisor_birth_panic_after_adoption_consumes_original_wait() {
    let (child, witness) = birth();
    let result = catch_unwind(AssertUnwindSafe(move || {
        let _owner = child;
        panic!("original checkpoint payload");
    }));
    assert_eq!(
        result.unwrap_err().downcast_ref::<&str>(),
        Some(&"original checkpoint payload")
    );
    assert_original_wait_consumed(&witness);
}

#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn supervisor_root_owner_keeps_bootstrap_active_without_private_completion() {
    use super::linux_supervisor_launcher::LinuxSupervisorLauncher;
    use crate::recovery_slot::{SlotError, SlotReservation};
    use std::sync::atomic::AtomicBool;

    let directory = tempfile::tempdir().unwrap();
    let slot_path = directory.path().join("original-bootstrap");
    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&slot_path)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let image = File::open(std::env::var_os("DG_LINUX_SUPERVISOR_ROLE").unwrap()).unwrap();
    let launcher = LinuxSupervisorLauncher::prepare(
        image,
        vec![CString::new("fixed-supervisor-role").unwrap()],
        Vec::new(),
        (23111, 23111),
    )
    .unwrap();
    let reservation = SlotReservation::acquire(file, deadline).unwrap();
    let mut owner = launcher.spawn(reservation, [7; 32], deadline).unwrap();
    let witness = duplicate(owner.child.as_ref().unwrap().pidfd_for_test());
    let cancel = AtomicBool::new(false);
    let mut output = Vec::new();
    loop {
        assert!(Instant::now() < deadline);
        if let Some(bytes) = owner.read_stdout().unwrap() {
            output.extend_from_slice(bytes);
            assert!(output.len() <= 4096);
        }
        owner.read_stderr().unwrap();
        assert!(!owner.poll_retirement(deadline).unwrap());
        match owner.poll_notification(&cancel) {
            Ok(crate::recovery_control::LinuxControlRead::Pending) => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => {
                assert_eq!(
                    error
                        .get_ref()
                        .unwrap()
                        .downcast_ref::<crate::recovery_control::ControlError>(),
                    Some(&crate::recovery_control::ControlError::Unconfirmed)
                );
                break;
            }
            other => panic!("silent role must not authenticate completion: {other:?}"),
        }
    }
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output).unwrap()["passed"],
        true
    );
    while !owner.poll_cleanup(deadline).unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_original_wait_consumed(&witness);
    assert!(owner.abort_unborn(deadline).is_err());
    assert_eq!(std::fs::read(&slot_path).unwrap(), b"DGSL01A\n");
    drop(owner);
    let file = File::options()
        .read(true)
        .write(true)
        .open(&slot_path)
        .unwrap();
    assert!(matches!(
        SlotReservation::acquire(file, deadline),
        Err(SlotError::Unconfirmed)
    ));
}

#[test]
#[ignore = "requires explicit compiled role and isolated root; invoked by native qualification"]
fn supervisor_root_owner_requires_original_wait_after_authenticated_completion() {
    use super::linux_supervisor_launcher::LinuxSupervisorLauncher;
    use crate::recovery_control::LinuxControlRead;
    use crate::recovery_slot::{LinuxSupervisorTrust, SlotReservation};
    use std::sync::atomic::AtomicBool;

    let directory = tempfile::tempdir().unwrap();
    let slot_path = directory.path().join("original-bootstrap");
    let deadline = Instant::now() + Duration::from_secs(5);
    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&slot_path)
        .unwrap();
    let reservation = SlotReservation::acquire(file, deadline).unwrap();
    let image = File::open(std::env::var_os("DG_LINUX_SUPERVISOR_ROLE").unwrap()).unwrap();
    let launcher = LinuxSupervisorLauncher::prepare(
        image,
        vec![
            CString::new("fixed-supervisor-role").unwrap(),
            CString::new("notify").unwrap(),
        ],
        Vec::new(),
        (23111, 23111),
    )
    .unwrap();
    let mut owner = launcher.spawn(reservation, [7; 32], deadline).unwrap();
    let witness = duplicate(owner.child.as_ref().unwrap().pidfd_for_test());
    let cancel = AtomicBool::new(false);
    let mut notifications = 0;
    loop {
        assert!(Instant::now() < deadline);
        match owner.poll_notification(&cancel).unwrap() {
            LinuxControlRead::Frame(_) => notifications += 1,
            LinuxControlRead::ConfirmedEof => break,
            LinuxControlRead::Pending => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    assert_eq!(notifications, 3);
    assert!(
        !owner.child.as_mut().unwrap().poll().unwrap(),
        "role held by original datagram barrier"
    );
    assert!(
        !owner.poll_retirement(deadline).unwrap(),
        "private completion cannot consume original wait early"
    );
    assert_eq!(std::fs::read(&slot_path).unwrap(), b"DGSL01A\n");
    let trust = LinuxSupervisorTrust::from_host(
        File::open(directory.path()).unwrap(),
        File::open("/proc/self/ns/user").unwrap(),
        File::open("/proc/self/ns/mnt").unwrap(),
    );
    owner.send_materials(&trust, &cancel).unwrap();
    let mut output = Vec::new();
    loop {
        assert!(Instant::now() < deadline);
        if let Some(bytes) = owner.read_stdout().unwrap() {
            output.extend_from_slice(bytes);
            assert!(output.len() <= 4096);
        }
        owner.read_stderr().unwrap();
        if owner.poll_retirement(deadline).unwrap() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_original_wait_consumed(&witness);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output).unwrap()["passed"],
        true
    );
    assert_eq!(std::fs::read(&slot_path).unwrap(), b"DGSL01C\n");
    let file = File::options()
        .read(true)
        .write(true)
        .open(&slot_path)
        .unwrap();
    SlotReservation::acquire(file, deadline)
        .unwrap()
        .abort_before_birth(deadline)
        .unwrap();
}
