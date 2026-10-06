//! 临时macOS CI内普通UID消费root安装的真实出生/协议夹具；不代表Engine发布验收。
use crate::macos_host_settings::MacosHostSettings;
use crate::macos_installation_lock::MacosInstallationLock;
use crate::native_child::MacosNativeLauncher;
use crate::scan_worker_child::ScanWorkerChild;
use crate::scan_worker_driver::ScanWorkerDriver;
use crate::scan_worker_error_projection::ScanWorkerErrorProjection;
use crate::scan_worker_failure::ScanWorkerFailure;
use crate::scan_worker_owned_failure::ScanWorkerOwnedFailure;
use crate::{EngineError, ScanWorkerHost, ScanWorkerRecovery, ScanWorkerRuntimeBudget};
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{DecodedTree, ExecutionOutcome, ProtocolLimits, WorkerRequest};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing explicit fixture input: {name}"))
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn installed_host() -> ScanWorkerHost {
    assert_ne!(unsafe { libc::getuid() }, 0);
    assert_eq!(unsafe { libc::getuid() }, unsafe { libc::geteuid() });
    assert_eq!(required("GITHUB_ACTIONS"), "true");
    assert_eq!(required("RUNNER_OS"), "macOS");
    assert_eq!(required("RUNNER_ENVIRONMENT"), "github-hosted");
    let run_id = required("GITHUB_RUN_ID");
    assert!(!run_id.is_empty() && run_id.bytes().all(|byte| byte.is_ascii_digit()));
    assert_eq!(required("DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE"), run_id);
    let limits = ProtocolLimits {
        max_frame_bytes: 1024 * 1024,
        max_stream_bytes: 8 * 1024 * 1024,
        max_nodes: 1024,
        max_depth: 32,
    };
    let host = ScanWorkerHost::from_installed_macos(
        ScanWorkerRuntimeBudget::new(limits, 64 * 1024, 1).unwrap(),
        deadline(),
        &mut || Ok(()),
    )
    .unwrap()
    .expect("root fixture must have installed an authorized image");
    let guard = MacosInstallationLock::acquire_shared(deadline(), &mut || Ok(())).unwrap();
    let settings = MacosHostSettings::read_authorized(&guard, deadline(), &mut || Ok(())).unwrap();
    assert_eq!(
        settings.active_epoch, 2,
        "consume the recovered root fixture generation"
    );
    let expected_hex = required("DISKGRAPH_MACOS_FIXTURE_SHA256");
    assert_eq!(expected_hex.len(), 64);
    assert!(expected_hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let mut expected = [0; 32];
    for (index, byte) in expected.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&expected_hex[index * 2..index * 2 + 2], 16).unwrap();
    }
    // 外层冻结摘要只是额外核对，Host仍仅从固定root保护材料取得信任。
    assert_eq!(settings.expected_sha256, expected);
    assert_eq!(
        settings.expected_bytes,
        required("DISKGRAPH_MACOS_FIXTURE_BYTES")
            .parse::<u64>()
            .unwrap()
    );
    drop(guard);
    host
}

fn fixture_tree(directory: &Path) -> std::path::PathBuf {
    // 本机及真实CI临时卷对非法UTF-8返回EILSEQ；扫描用可创建的Unicode名称。
    // 协议原生字节无损合同由独立非文件系统测试覆盖，不能声称此卷支持非法文件名。
    let root = directory.join("scope-目录");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("alpha"), b"abc").unwrap();
    std::fs::write(root.join("sub/beta"), b"abcde").unwrap();
    std::fs::write(root.join("leaf-文件"), b"abcdefg").unwrap();
    root
}

// stop_after_birth: None正常运行，Some(false)原检查点取消，Some(true)原panic。
// 所有唯一owner及Recovery均在catch外；任何断言只能在实际处置/保留之后执行。
fn run_installed(
    host: &ScanWorkerHost,
    root: &Path,
    stop_after_birth: Option<bool>,
) -> Result<DecodedTree, EngineError> {
    let expires = deadline();
    let recovery = ScanWorkerRecovery::new(Arc::clone(&host.registry));
    let reservation = host.registry.reserve()?;
    let mut launched: Option<ScanWorkerChild> = None;
    let mut driver: Option<ScanWorkerDriver> = None;
    let observed = catch_unwind(AssertUnwindSafe(|| -> Result<DecodedTree, EngineError> {
        let permit = host.prepare_macos_installation(expires, &mut || Ok(()))?;
        let launcher = MacosNativeLauncher::prepare_qualified(permit, expires, &mut || Ok(()))?;
        launcher.spawn_into(&mut launched, expires, &mut || Ok(()))?;
        let options = ScanOptions {
            apparent_size: true,
            ..ScanOptions::default()
        };
        let request = WorkerRequest::scan(root, &options, host.budget.response_limits())?;
        driver = Some(
            ScanWorkerDriver::new(
                &mut launched,
                request,
                (env!("DISKGRAPH_ENGINE_TARGET"), PIN),
                expires,
                host.budget.stderr_bytes(),
            )
            .map_err(|error| ScanWorkerErrorProjection::driver(error, |never| match never {}))?,
        );
        loop {
            let active = driver
                .as_mut()
                .expect("driver remains outside unwind boundary");
            let result = active
                .poll(false, || {
                    if let Some(panic) = stop_after_birth {
                        if panic {
                            std::panic::panic_any(String::from("installed helper original panic"));
                        }
                        return Err(EngineError::Poisoned);
                    }
                    host.authorize_macos_epoch(expires, &mut || Ok(()))
                        .map(|_| ())
                })
                .map_err(|error| ScanWorkerErrorProjection::driver(error, |primary| primary))?;
            if let Some(result) = result {
                return match result {
                    ExecutionOutcome::Tree(tree) => {
                        // 该事实来自原Driver实际normal wait，不以EOF或cleanup替代。
                        assert_eq!(active.exit_code(), Some(0));
                        Ok(tree)
                    }
                    ExecutionOutcome::Failure(failure) => {
                        Err(ScanWorkerErrorProjection::remote(failure))
                    }
                };
            }
            std::thread::sleep(
                expires
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20)),
            );
        }
    }));
    let succeeded = matches!(&observed, Ok(Ok(_)));
    if !succeeded {
        if let Some(active) = driver.take() {
            let cleanup = active.unwind_cleanup_error().cloned();
            let (_, owner) = active
                .into_owned_failure(ScanWorkerFailure::<std::convert::Infallible>::Stopped {
                    cleanup,
                })
                .into_parts();
            if let Some(owner) = owner {
                reservation.retain(owner);
            }
        }
        if let Some(owner) = launched.take() {
            let (_, owner) = ScanWorkerOwnedFailure::dispose(
                ScanWorkerFailure::<std::convert::Infallible>::Stopped { cleanup: None },
                owner,
            )
            .into_parts();
            if let Some(owner) = owner {
                reservation.retain(owner);
            }
        }
    }
    drop(driver);
    drop(reservation);
    let drained = recovery.drain();
    // 若OS拒绝真实回收，不能因测试panic丢失最后Recovery；进程内保留责任并使测试失败。
    if !matches!(drained, Ok(true)) {
        std::mem::forget(recovery);
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!("installed fixture cleanup did not complete; Recovery retained");
    }
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    match observed {
        Ok(result) => result,
        Err(payload) => resume_unwind(payload),
    }
}

#[test]
#[ignore = "requires ordinary UID ephemeral CI installed fixture"]
fn macos_installed_helper_returns_complete_tree_after_real_normal_wait() {
    let host = installed_host();
    let directory = tempfile::tempdir().unwrap();
    // 只见证此托管CI临时卷的实际拒绝，不推断所有macOS文件系统的文件名能力。
    let invalid = directory.path().join(OsStr::from_bytes(b"scope-\xff"));
    let rejected = std::fs::create_dir(&invalid).unwrap_err();
    assert_eq!(rejected.raw_os_error(), Some(libc::EILSEQ));
    let root = fixture_tree(directory.path());
    let tree = run_installed(&host, &root, None).unwrap();
    assert_eq!(tree.bytes, 15);
    assert_eq!(tree.files, 3);
    assert_eq!(tree.dirs, 2);
    let mut pending = vec![&*tree];
    let mut nodes = 0;
    let mut unicode_leaf_displayed = false;
    while let Some(node) = pending.pop() {
        nodes += 1;
        assert!(!node.read_error);
        if node.name.as_ref() == "leaf-文件" {
            unicode_leaf_displayed = true;
            assert_eq!(node.bytes, 7);
        }
        pending.extend(node.children.iter());
    }
    assert_eq!(nodes, 5);
    assert!(
        unicode_leaf_displayed,
        "real installed helper must preserve the supported Unicode leaf name"
    );
    assert_eq!(host.registry.occupied().unwrap(), 0);
    println!(
        "DISKGRAPH_MACOS_INSTALLED_FIXTURE complete_tree nodes=5 files=3 bytes=15 exit=0 epoch=2"
    );
}

#[test]
fn macos_worker_request_roundtrip_preserves_non_utf8_native_path_without_filesystem() {
    // 不创建该路径、不调用扫描器：Unix协议字节范围不能被APFS/HFS名称准入缩窄。
    let original = Path::new(OsStr::from_bytes(b"/scope-\xff/nested-\xfe"));
    let options = ScanOptions::default();
    let limits = ProtocolLimits {
        max_frame_bytes: 1024 * 1024,
        max_stream_bytes: 8 * 1024 * 1024,
        max_nodes: 1024,
        max_depth: 32,
    };
    let request = WorkerRequest::scan(original, &options, limits).unwrap();
    let wire = serde_json::to_vec(&request).unwrap();
    let decoded: WorkerRequest = serde_json::from_slice(&wire).unwrap();
    let (restored, _, _) = decoded.into_scan().unwrap();
    assert_eq!(
        restored.as_os_str().as_bytes(),
        original.as_os_str().as_bytes()
    );
    assert_ne!(
        restored.as_os_str().as_bytes(),
        original.to_string_lossy().as_bytes()
    );
}

#[test]
#[ignore = "requires ordinary UID ephemeral CI installed fixture"]
fn macos_installed_helper_original_cancel_after_birth_is_reaped() {
    let host = installed_host();
    let directory = tempfile::tempdir().unwrap();
    let root = fixture_tree(directory.path());
    assert!(matches!(
        run_installed(&host, &root, Some(false)),
        Err(EngineError::Poisoned)
    ));
    assert_eq!(host.registry.occupied().unwrap(), 0);
    assert!(matches!(
        host.prepare_macos_installation(deadline(), &mut || Err(EngineError::Poisoned)),
        Err(EngineError::Poisoned)
    ));
}

#[test]
#[ignore = "requires ordinary UID ephemeral CI installed fixture"]
fn macos_installed_helper_original_panic_after_birth_keeps_recovery_responsibility() {
    let host = installed_host();
    let directory = tempfile::tempdir().unwrap();
    let root = fixture_tree(directory.path());
    let payload =
        catch_unwind(AssertUnwindSafe(|| run_installed(&host, &root, Some(true)))).unwrap_err();
    assert_eq!(
        *payload.downcast::<String>().unwrap(),
        "installed helper original panic"
    );
    assert_eq!(host.registry.occupied().unwrap(), 0);
}
