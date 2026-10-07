//! 可信本地容量域的真实锁与异常记录验收；不证明生产 launcher 已接通。
use crate::TrustedLocalRecoveryDomain;
use crate::recovery_slot::SlotError;
use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

fn private_directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn domain(path: &std::path::Path) -> TrustedLocalRecoveryDomain {
    TrustedLocalRecoveryDomain::from_host(File::open(path).unwrap(), deadline()).unwrap()
}

#[test]
fn four_original_slots_refuse_fifth_and_abnormal_drop_does_not_reuse_active() {
    let dir = private_directory();
    let domain = domain(dir.path());
    let mut active = Vec::new();
    for _ in 0..4 {
        active.push(
            domain
                .reserve(deadline())
                .unwrap()
                .activate(deadline())
                .unwrap(),
        );
    }
    assert!(matches!(domain.reserve(deadline()), Err(SlotError::Busy)));
    drop(active);
    assert!(matches!(
        observe_unconfirmed(&domain),
        Err(SlotError::Unconfirmed)
    ));
    for index in 0..4 {
        assert_eq!(
            std::fs::read(dir.path().join(format!("supervisor_{index}.slot"))).unwrap(),
            b"DGSL01A\n"
        );
    }
}

#[test]
fn prebirth_abort_reuses_the_same_slot() {
    let dir = private_directory();
    let domain = domain(dir.path());
    domain
        .reserve(deadline())
        .unwrap()
        .abort_before_birth(deadline())
        .unwrap();
    domain
        .reserve(deadline())
        .unwrap()
        .abort_before_birth(deadline())
        .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("supervisor_0.slot")).unwrap(),
        b"DGSL01C\n"
    );
}

#[test]
fn symlink_multilink_and_changed_domain_permissions_are_refused() {
    let dir = private_directory();
    let domain = domain(dir.path());
    let outside = tempfile::NamedTempFile::new().unwrap();
    let slot = dir.path().join("supervisor_0.slot");
    std::os::unix::fs::symlink(outside.path(), &slot).unwrap();
    assert!(domain.reserve(deadline()).is_err());
    std::fs::remove_file(&slot).unwrap();
    std::fs::hard_link(outside.path(), &slot).unwrap();
    assert!(matches!(
        domain.reserve(deadline()),
        Err(SlotError::Unsupported)
    ));
    std::fs::remove_file(&slot).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        domain.reserve(deadline()),
        Err(SlotError::Unsupported)
    ));
}

#[test]
fn expired_admission_does_not_create_a_slot() {
    let dir = private_directory();
    let domain = domain(dir.path());
    assert!(matches!(
        domain.reserve(Instant::now() - Duration::from_secs(1)),
        Err(SlotError::Deadline)
    ));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_real_child_competes_in_the_same_four_slot_domain() {
    let dir = private_directory();
    let domain = domain(dir.path());
    let mut original = Vec::new();
    for _ in 0..3 {
        original.push(
            domain
                .reserve(deadline())
                .unwrap()
                .activate(deadline())
                .unwrap(),
        );
    }
    let stdout = tempfile::NamedTempFile::new().unwrap();
    let stderr = tempfile::NamedTempFile::new().unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    let mut child = OriginalFixtureChild::spawn(dir.path(), &stdout, &stderr, false).unwrap();
    let status = child.wait_until(until).unwrap();
    assert!(status.success(), "child failed: {}", read_log(&stderr));
    let stdout = read_log(&stdout);
    assert!(stdout.contains("DG_LOCAL_DOMAIN_ORIGINAL_FOURTH_ACTIVE=1"));
    assert!(stdout.contains("1 passed; 0 failed; 0 ignored"));
    assert_eq!(
        std::fs::read(dir.path().join("supervisor_3.slot")).unwrap(),
        b"DGSL01A\n"
    );
    assert!(matches!(
        observe_unconfirmed(&domain),
        Err(SlotError::Unconfirmed)
    ));
    drop(original);
    assert!(matches!(
        observe_unconfirmed(&domain),
        Err(SlotError::Unconfirmed)
    ));
}

#[test]
#[ignore = "native subprocess fixture; invoked explicitly by parent with isolated domain"]
fn native_child_reserves_fourth_original_slot() {
    let path = std::path::PathBuf::from(
        std::env::var_os("DISKGRAPH_TEST_LOCAL_RECOVERY_DOMAIN")
            .expect("explicit native fixture domain"),
    );
    let domain = domain(&path);
    let _active = domain
        .reserve(deadline())
        .unwrap()
        .activate(deadline())
        .unwrap();
    assert_eq!(
        std::fs::read(path.join("supervisor_3.slot")).unwrap(),
        b"DGSL01A\n"
    );
    println!("DG_LOCAL_DOMAIN_ORIGINAL_FOURTH_ACTIVE=1");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    if std::env::var_os("DISKGRAPH_TEST_LOCAL_DOMAIN_HOLD").is_some() {
        std::thread::sleep(Duration::from_secs(30));
    }
}

fn observe_unconfirmed(
    domain: &TrustedLocalRecoveryDomain,
) -> Result<crate::recovery_slot::SlotReservation, SlotError> {
    let until = deadline();
    loop {
        match domain.reserve(until) {
            Err(SlotError::Busy) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            outcome => return outcome,
        }
    }
}

fn read_log(log: &tempfile::NamedTempFile) -> String {
    use std::io::Read;
    let mut bytes = Vec::new();
    File::open(log.path())
        .unwrap()
        .take(65537)
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(
        bytes.len() <= 65536,
        "native fixture log exceeded diagnostic budget"
    );
    String::from_utf8(bytes).unwrap()
}

/// 测试实际子进程的唯一 owner；所有失败/展开路径均终止并 wait，不丢弃原 Child。
struct OriginalFixtureChild {
    child: Option<std::process::Child>,
}

impl OriginalFixtureChild {
    fn spawn(
        root: &std::path::Path,
        stdout: &tempfile::NamedTempFile,
        stderr: &tempfile::NamedTempFile,
        hold: bool,
    ) -> std::io::Result<Self> {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "trusted_local_recovery_domain_tests::native_child_reserves_fourth_original_slot",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("DISKGRAPH_TEST_LOCAL_RECOVERY_DOMAIN", root)
            .env_remove("DISKGRAPH_TEST_LOCAL_DOMAIN_HOLD")
            .stdin(std::process::Stdio::null())
            .stdout(stdout.as_file().try_clone()?)
            .stderr(stderr.as_file().try_clone()?);
        if hold {
            command.env("DISKGRAPH_TEST_LOCAL_DOMAIN_HOLD", "1");
        }
        Ok(Self {
            child: Some(command.spawn()?),
        })
    }

    fn wait_until(&mut self, until: Instant) -> std::io::Result<std::process::ExitStatus> {
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .expect("original fixture child")
                .try_wait()?
            {
                self.child.take();
                return Ok(status);
            }
            if Instant::now() >= until {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "native fixture observation expired; original child retained",
                ));
            }
            std::thread::sleep(
                until
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(2)),
            );
        }
    }

    fn terminate_and_wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let child = self.child.as_mut().expect("original fixture child");
        child.kill()?;
        let status = child.wait()?;
        self.child.take();
        Ok(status)
    }
}

impl Drop for OriginalFixtureChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            if let Err(error) = child.wait() {
                eprintln!("original native fixture wait failed: {error}");
            }
        }
    }
}

#[test]
fn observed_child_timeout_is_reaped_and_keeps_its_active_slot_unconfirmed() {
    let dir = private_directory();
    let domain = domain(dir.path());
    let mut original = Vec::new();
    for _ in 0..3 {
        original.push(
            domain
                .reserve(deadline())
                .unwrap()
                .activate(deadline())
                .unwrap(),
        );
    }
    let stdout = tempfile::NamedTempFile::new().unwrap();
    let stderr = tempfile::NamedTempFile::new().unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    let mut child = OriginalFixtureChild::spawn(dir.path(), &stdout, &stderr, true).unwrap();
    while !read_log(&stdout).contains("DG_LOCAL_DOMAIN_ORIGINAL_FOURTH_ACTIVE=1") {
        assert!(
            Instant::now() < until,
            "original child did not reach ACTIVE: {}",
            read_log(&stderr)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let error = child
        .wait_until(Instant::now() + Duration::from_millis(20))
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    let status = child.terminate_and_wait().unwrap();
    assert!(!status.success());
    assert_eq!(
        std::fs::read(dir.path().join("supervisor_3.slot")).unwrap(),
        b"DGSL01A\n"
    );
    assert!(matches!(
        observe_unconfirmed(&domain),
        Err(SlotError::Unconfirmed)
    ));
    drop(original);
}
