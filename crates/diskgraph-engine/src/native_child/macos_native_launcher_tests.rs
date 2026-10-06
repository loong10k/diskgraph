//! 原生启动的外部 owner 合同；系统镜像测试声明不代替可信 fresh 安装验收。
use super::macos_native_launcher::MacosNativeLauncher;
use crate::ScanWorkerHostConfig;
use crate::macos_filesystem_state::MacosFilesystemState;
use crate::macos_installation_claims::MacosInstallationClaims;
use crate::macos_installation_lease::MacosInstallationLease;
use crate::macos_installation_trust::MacosInstallationTrust;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn lease(deadline: Instant) -> Arc<MacosInstallationLease> {
    let path = Path::new("/bin/cat");
    let state = MacosFilesystemState::capture(&File::open(path).unwrap(), false).unwrap();
    let digest: [u8; 32] = Sha256::digest(fs::read(path).unwrap()).into();
    let expected = ScanWorkerHostConfig::from_expected_image(digest, state.len).unwrap();
    let claims = MacosInstallationClaims {
        schema_version: 1,
        installation_id: [8; 16],
        epoch: 9,
        policy_version: 1,
        target: env!("DISKGRAPH_ENGINE_TARGET").into(),
        protocol_version: 2,
        pinned_scanner_revision: "158f9cc2f0b332194a3ffc5acec47760c99146d8".into(),
        image_sha256: digest,
        image_bytes: state.len,
        native_path: path.as_os_str().as_bytes().to_vec(),
        volume_uuid: state.volume_uuid,
        fsid: state.fsid,
        device: state.device,
        inode: state.inode,
        birth_seconds: state.birth_seconds,
        birth_nanoseconds: state.birth_nanoseconds,
        fresh_from_birth: true,
    };
    let key = SigningKey::from_bytes(&[82; 32]);
    let signature = key.sign(&claims.signing_message()).to_bytes().to_vec();
    let receipt =
        serde_json::to_vec(&serde_json::json!({"claims":claims,"signature":signature})).unwrap();
    let trust =
        MacosInstallationTrust::from_host(key.verifying_key().to_bytes(), 9, Path::new("/bin"))
            .unwrap();
    Arc::new(
        MacosInstallationLease::admit(&receipt, &trust, &expected, deadline, &mut || Ok(()))
            .unwrap(),
    )
}

#[test]
fn native_birth_keeps_live_owner_outside_postcheck_and_reaps_original_process() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let launcher = MacosNativeLauncher::prepare(lease(deadline), deadline, &mut || Ok(())).unwrap();
    let mut owner = None;
    let born = std::cell::Cell::new(false);
    let original_pid = std::cell::Cell::new(0);
    let result = launcher.spawn_observed(
        &mut owner,
        deadline,
        &mut || {
            if born.get() {
                Err(crate::EngineError::Poisoned)
            } else {
                Ok(())
            }
        },
        |pid| {
            // 测试观察点只能在原生出生后触发，检查实际 SID/PGID，不依赖回调次数猜测。
            assert_eq!(unsafe { libc::getsid(pid as i32) }, pid as i32);
            assert_eq!(unsafe { libc::getpgid(pid as i32) }, pid as i32);
            original_pid.set(pid);
            born.set(true);
        },
    );
    assert!(matches!(result, Err(crate::EngineError::Poisoned)));
    assert!(born.get());
    let child = owner
        .as_mut()
        .expect("original child remains in caller slot");
    assert!(!child.poll().unwrap());
    child.cleanup().unwrap();
    child.cleanup().unwrap();
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(original_pid.get() as i32, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[test]
fn fixed_native_cat_stream_normal_wait_and_lease_survive_until_owner_release() {
    use super::ControlWriteStatus;
    let deadline = Instant::now() + Duration::from_secs(10);
    let installation = lease(deadline);
    let weak = Arc::downgrade(&installation);
    let launcher = MacosNativeLauncher::prepare(installation, deadline, &mut || Ok(())).unwrap();
    let mut owner = None;
    launcher
        .spawn_into(&mut owner, deadline, &mut || Ok(()))
        .unwrap();
    assert!(weak.upgrade().is_some());
    let child = owner.as_mut().unwrap();
    let message = b"native lease stream\n";
    let mut sent = 0;
    while sent < message.len() {
        assert!(Instant::now() < deadline);
        let mut result = child.start_control_write(&message[sent..]).unwrap();
        while result == ControlWriteStatus::Pending {
            assert!(Instant::now() < deadline);
            result = child.poll_control_write().unwrap();
        }
        let ControlWriteStatus::Written(count) = result else {
            panic!("control unexpectedly closed");
        };
        assert!(count > 0);
        sent += count;
    }
    child.request_control_close().unwrap();
    let mut stdout = Vec::new();
    loop {
        assert!(Instant::now() < deadline);
        if let Some(bytes) = child.read_stdout().unwrap() {
            stdout.extend_from_slice(bytes);
        }
        if let Some(bytes) = child.read_stderr().unwrap() {
            assert!(bytes.is_empty());
        }
        assert!(stdout.len() <= message.len());
        if child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(stdout, message);
    assert_eq!(child.exit_code(), Some(0));
    assert!(weak.upgrade().is_some());
    owner.take();
    assert!(weak.upgrade().is_none());
}

#[test]
fn panic_after_birth_keeps_original_payload_and_owner_for_outer_cleanup() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let installation = lease(deadline);
    let weak = Arc::downgrade(&installation);
    let launcher = MacosNativeLauncher::prepare(installation, deadline, &mut || Ok(())).unwrap();
    let mut owner = None;
    let payload = Box::new(String::from("original native birth panic"));
    let address = (&*payload as *const String) as usize;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        launcher.spawn_observed(&mut owner, deadline, &mut || Ok(()), |_| {
            std::panic::panic_any(payload)
        })
    }));
    let original = result.unwrap_err().downcast::<Box<String>>().unwrap();
    assert_eq!((&**original as *const String) as usize, address);
    assert!(weak.upgrade().is_some());
    let child = owner.as_mut().expect("panic keeps original external child");
    assert!(!child.poll().unwrap());
    child.cleanup().unwrap();
    owner.take();
    assert!(weak.upgrade().is_none());
}

#[test]
fn expired_prebirth_request_does_not_fill_external_owner() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let launcher = MacosNativeLauncher::prepare(lease(deadline), deadline, &mut || Ok(())).unwrap();
    let mut owner = None;
    let result = launcher.spawn_into(
        &mut owner,
        Instant::now() - Duration::from_secs(1),
        &mut || Ok(()),
    );
    assert!(matches!(
        result,
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::BudgetExceeded
        ))
    ));
    assert!(owner.is_none());
}

#[test]
fn occupied_birth_gate_cancels_native_launch_before_external_owner_is_filled() {
    use super::native_birth_gate::{NativeBirthGate, set_wait_hook};
    use std::sync::atomic::{AtomicBool, Ordering};
    let deadline = Instant::now() + Duration::from_secs(10);
    let launcher = MacosNativeLauncher::prepare(lease(deadline), deadline, &mut || Ok(())).unwrap();
    let held = NativeBirthGate::acquire(&mut || Ok::<(), ()>(())).unwrap();
    let thread = std::thread::spawn(move || {
        let waiting = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&waiting);
        set_wait_hook(Box::new(move || observed.store(true, Ordering::SeqCst)));
        let mut owner = None;
        let result = launcher.spawn_into(&mut owner, deadline, &mut || {
            if waiting.load(Ordering::SeqCst) {
                Err(crate::EngineError::Poisoned)
            } else {
                Ok(())
            }
        });
        assert!(waiting.load(Ordering::SeqCst));
        assert!(matches!(result, Err(crate::EngineError::Poisoned)));
        assert!(owner.is_none());
    });
    thread.join().unwrap();
    drop(held);
}

#[test]
fn qualified_native_birth_holds_update_lock_until_real_birth_and_releases_on_panic() {
    use crate::macos_installation_lock::MacosInstallationLock;
    use crate::macos_spawn_permit::MacosSpawnPermit;
    use std::os::fd::AsRawFd;
    let deadline = Instant::now() + Duration::from_secs(10);
    let file = tempfile::NamedTempFile::new().unwrap();
    let guard =
        MacosInstallationLock::test_shared(File::open(file.path()).unwrap(), deadline, &mut || {
            Ok(())
        })
        .unwrap();
    let installation = lease(deadline);
    let weak = Arc::downgrade(&installation);
    let permit = MacosSpawnPermit::new(installation, guard);
    let launcher =
        MacosNativeLauncher::prepare_qualified(permit, deadline, &mut || Ok(())).unwrap();
    let contender = File::open(file.path()).unwrap();
    assert_eq!(
        unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EWOULDBLOCK)
    );
    let mut owner = None;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        launcher
            .spawn_observed(&mut owner, deadline, &mut || Ok(()), |pid| {
                assert!(pid > 0);
                assert_eq!(
                    unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                    0
                );
                std::panic::panic_any(String::from("qualified birth panic"));
            })
            .unwrap();
    }))
    .unwrap_err();
    assert_eq!(
        *panic.downcast::<String>().unwrap(),
        "qualified birth panic"
    );
    assert!(weak.upgrade().is_some());
    owner
        .as_mut()
        .expect("original live owner survives")
        .cleanup()
        .unwrap();
    owner.take();
    assert!(weak.upgrade().is_none());
}
