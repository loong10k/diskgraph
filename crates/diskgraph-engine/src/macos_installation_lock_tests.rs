use crate::EngineError;
use crate::macos_installation_lock::MacosInstallationLock;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
#[test]
fn exclusive_fixture() {
    let Some(path) = std::env::var_os("DG_MACOS_INSTALL_LOCK_FIXTURE") else {
        return;
    };
    let file = File::open(path).unwrap();
    assert_eq!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    std::io::stdout().write_all(b"locked\n").unwrap();
    std::io::stdout().flush().unwrap();
    let mut signal = [0_u8; 1];
    std::io::stdin().read_exact(&mut signal).unwrap();
}
fn with_exclusive(test: impl FnOnce(&std::path::Path)) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "macos_installation_lock_tests::exclusive_fixture",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("DG_MACOS_INSTALL_LOCK_FIXTURE", file.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let fd = reader.get_ref().as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut line = String::new();
        loop {
            match reader.read_line(&mut line) {
                Ok(0) => panic!("fixture exited before actual exclusive lock"),
                Ok(_) if line.contains("locked") => break,
                Ok(_) => line.clear(),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => panic!("fixture ready read failed: {error}"),
            }
            assert!(Instant::now() < deadline, "fixture ready deadline expired");
            std::thread::sleep(Duration::from_millis(2));
        }
        test(file.path());
    }));
    let release = if outcome.is_ok() {
        child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("missing fixture input"))
            .and_then(|mut input| input.write_all(b"x"))
    } else {
        Ok(())
    };
    if outcome.is_err() || release.is_err() {
        let _ = child.kill();
    }
    let waited = child.wait();
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
    assert!(release.is_ok(), "fixture release failed: {release:?}");
    assert!(waited.unwrap().success());
}
#[test]
fn real_cross_process_contention_preserves_original_cancel() {
    with_exclusive(|path| {
        let mut checks = 0;
        let result = MacosInstallationLock::test_shared(
            File::open(path).unwrap(),
            Instant::now() + Duration::from_secs(10),
            &mut || {
                checks += 1;
                if checks == 1 {
                    Ok(())
                } else {
                    Err(EngineError::Poisoned)
                }
            },
        );
        assert!(matches!(result, Err(EngineError::Poisoned)));
    });
}
#[test]
fn real_cross_process_contention_expires_at_original_deadline() {
    with_exclusive(|path| {
        let result = MacosInstallationLock::test_shared(
            File::open(path).unwrap(),
            Instant::now() + Duration::from_millis(20),
            &mut || Ok(()),
        );
        assert!(matches!(
            result,
            Err(EngineError::Business(
                diskgraph_core::BusinessError::BudgetExceeded
            ))
        ));
    });
}
#[test]
fn dropping_shared_guard_releases_same_kernel_lock_without_clone() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let guard = MacosInstallationLock::test_shared(
        File::open(file.path()).unwrap(),
        Instant::now() + Duration::from_secs(10),
        &mut || Ok(()),
    )
    .unwrap();
    let contender = File::open(file.path()).unwrap();
    assert_eq!(
        unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EWOULDBLOCK)
    );
    drop(guard);
    assert_eq!(
        unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
}

#[test]
fn updater_lock_preserves_original_cancel_and_rejects_unprivileged_identity() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    assert!(matches!(
        crate::macos_installation_lock::MacosInstallationLock::acquire_exclusive(
            deadline,
            &mut || Err(crate::EngineError::Poisoned)
        ),
        Err(crate::EngineError::Poisoned)
    ));
    if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
        assert!(matches!(
            crate::macos_installation_lock::MacosInstallationLock::acquire_exclusive(
                deadline,
                &mut || Ok(())
            ),
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::Unsupported
            ))
        ));
    }
}
