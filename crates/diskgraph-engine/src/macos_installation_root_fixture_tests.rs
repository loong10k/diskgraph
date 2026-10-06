//! 仅临时托管macOS CI显式运行的真实root发行夹具，不由环境缺失产生假PASS。
//! 本文件不启用Runtime；普通UID宿主与跨进程消费由外层资格流程独立验证。
use crate::macos_epoch_floor::MacosEpochFloor;
use crate::macos_host_settings::MacosHostSettings;
use crate::macos_installation_bootstrap::MacosInstallationBootstrap;
use crate::macos_installation_lock::MacosInstallationLock;
use crate::macos_installation_publisher::MacosInstallationPublisher;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use ed25519_dalek::SigningKey;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

const BASE: &str = "/Library/Application Support/DiskGraph/scan-worker";

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing explicit fixture input: {name}"))
}

fn decode_sha256(value: &str) -> [u8; 32] {
    assert_eq!(value.len(), 64, "outer harness must freeze the full SHA256");
    assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let mut bytes = [0; 32];
    for (index, output) in bytes.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    bytes
}

fn authorized_epoch(expected: u64) {
    let guard = MacosInstallationLock::acquire_shared(deadline(), &mut || Ok(())).unwrap();
    let settings = MacosHostSettings::read_authorized(&guard, deadline(), &mut || Ok(())).unwrap();
    assert_eq!(settings.active_epoch, expected);
}

fn record(stage: &str, epoch: u64, id: [u8; 16], key: [u8; 32], sha: [u8; 32], bytes: u64) {
    println!(
        "DISKGRAPH_MACOS_ROOT_FIXTURE {}",
        serde_json::json!({
            "schema_version": 1, "stage": stage, "epoch": epoch,
            "installation_id": id, "public_key": key,
            "expected_sha256": sha, "expected_bytes": bytes,
            "root": BASE, "target": env!("DISKGRAPH_ENGINE_TARGET"),
            "run_id": required("GITHUB_RUN_ID")
        })
    );
}

#[test]
#[ignore = "requires root ephemeral macOS CI"]
fn root_fresh_install_update_interruption_and_floor_bound_recovery() {
    assert_eq!(unsafe { libc::getuid() }, 0, "real UID must be root");
    assert_eq!(unsafe { libc::geteuid() }, 0, "effective UID must be root");
    assert_eq!(required("GITHUB_ACTIONS"), "true");
    assert_eq!(required("RUNNER_OS"), "macOS");
    assert_eq!(required("RUNNER_ENVIRONMENT"), "github-hosted");
    let run_id = required("GITHUB_RUN_ID");
    assert!(!run_id.is_empty() && run_id.bytes().all(|byte| byte.is_ascii_digit()));
    assert_eq!(required("DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE"), run_id);
    // 环境只是显式执行护栏，不是生产信任根；已有系统布局绝不覆盖或清理。
    match std::fs::symlink_metadata(BASE) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        other => panic!("fixture requires a previously absent installation base: {other:?}"),
    }
    let source = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(required("DISKGRAPH_MACOS_FIXTURE_HELPER"))
        .unwrap();
    let sha = decode_sha256(&required("DISKGRAPH_MACOS_FIXTURE_SHA256"));
    let bytes: u64 = required("DISKGRAPH_MACOS_FIXTURE_BYTES").parse().unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image(sha, bytes).unwrap();
    assert!(source.metadata().unwrap().is_file());
    assert_eq!(source.metadata().unwrap().len(), bytes);
    let mut entropy = [0; 64];
    File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut entropy)
        .unwrap();
    let key = SigningKey::from_bytes(entropy[..32].try_into().unwrap());
    let first_id: [u8; 16] = entropy[32..48].try_into().unwrap();
    let second_id: [u8; 16] = entropy[48..].try_into().unwrap();
    assert_ne!(first_id, [0; 16]);
    assert_ne!(second_id, [0; 16]);
    assert_ne!(first_id, second_id);
    MacosInstallationBootstrap::prepare(deadline(), &mut || Ok(())).unwrap();
    MacosInstallationPublisher::publish(
        &source,
        &expected,
        &key,
        1,
        first_id,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    authorized_epoch(1);
    // 首装后恢复也是实调用：没有pending时只能持久确认已匹配的active。
    MacosInstallationPublisher::recover(deadline(), &mut || Ok(())).unwrap();
    record(
        "baseline_installed",
        1,
        first_id,
        key.verifying_key().to_bytes(),
        sha,
        bytes,
    );
    let floor_path = Path::new(BASE).join("epoch-floor.json");
    let active_path = Path::new(BASE).join("active.json");
    let old_floor = std::fs::read(&floor_path).unwrap();
    let old_active = std::fs::read(&active_path).unwrap();
    let old_floor_inode = std::fs::metadata(&floor_path).unwrap().ino();
    let mut interrupted = false;
    let result = MacosInstallationPublisher::publish(
        &source,
        &expected,
        &key,
        2,
        second_id,
        deadline(),
        &mut || {
            let current = std::fs::read(&floor_path)?;
            if current != old_floor {
                assert_eq!(MacosEpochFloor::decode(&current)?.epoch(), 2);
                assert_eq!(std::fs::read(&active_path)?, old_active);
                interrupted = true;
                return Err(EngineError::Poisoned);
            }
            Ok(())
        },
    );
    assert!(
        interrupted,
        "must reach actual floor rename before active publication"
    );
    assert!(matches!(result, Err(EngineError::Poisoned)));
    let committed_floor = std::fs::read(&floor_path).unwrap();
    let committed_inode = std::fs::metadata(&floor_path).unwrap().ino();
    assert_ne!(committed_inode, old_floor_inode);
    assert_eq!(std::fs::read(&active_path).unwrap(), old_active);
    {
        let guard = MacosInstallationLock::acquire_shared(deadline(), &mut || Ok(())).unwrap();
        assert!(matches!(
            MacosHostSettings::read_authorized(&guard, deadline(), &mut || Ok(())),
            Err(EngineError::Business(BusinessError::Conflict))
        ));
    }
    record(
        "floor_renamed_active_rejected",
        2,
        second_id,
        key.verifying_key().to_bytes(),
        sha,
        bytes,
    );
    MacosInstallationPublisher::recover(deadline(), &mut || Ok(())).unwrap();
    authorized_epoch(2);
    assert_eq!(std::fs::read(&floor_path).unwrap(), committed_floor);
    assert_eq!(
        std::fs::metadata(&floor_path).unwrap().ino(),
        committed_inode
    );
    let recovered_active = std::fs::read(&active_path).unwrap();
    assert_ne!(recovered_active, old_active);
    for rejected_epoch in [2, 1] {
        let result = MacosInstallationPublisher::publish(
            &source,
            &expected,
            &key,
            rejected_epoch,
            first_id,
            deadline(),
            &mut || Ok(()),
        );
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::Conflict))
        ));
        assert_eq!(std::fs::read(&floor_path).unwrap(), committed_floor);
        assert_eq!(
            std::fs::metadata(&floor_path).unwrap().ino(),
            committed_inode
        );
        assert_eq!(std::fs::read(&active_path).unwrap(), recovered_active);
    }
    record(
        "recovered_monotonic_ready_for_ordinary_host",
        2,
        second_id,
        key.verifying_key().to_bytes(),
        sha,
        bytes,
    );
    // 有意保留真实root材料供另一个ordinary进程消费；不在Drop删除系统目录。
}
