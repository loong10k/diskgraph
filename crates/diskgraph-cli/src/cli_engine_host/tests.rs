//! 原 panic payload 与空恢复责任边界；来源：PF-06，不伪造原生进程 owner。
use super::CliEngineHost;
use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerHost, ScanWorkerHostConfig,
    ScanWorkerRuntimeBudget,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::sync::Arc;

#[test]
fn expired_original_startup_deadline_refuses_database_birth() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("not_born");
    let result = CliEngineHost::open_until(
        EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        },
        std::time::Instant::now(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::BudgetExceeded
        ))
    ));
    assert!(
        !data.exists(),
        "expired original admission created database state"
    );
}

#[test]
fn command_panic_preserves_original_payload_with_external_empty_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let session = managed_empty_host(&directory);
    let survivor = Arc::clone(&session.engine);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.execute(|_| -> Result<(), EngineError> {
            std::panic::panic_any("cli-original-payload")
        })
    }));
    let payload = result.unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"cli-original-payload")
    );
    assert_original_registry_sealed(&survivor, &directory);
}

fn admitted_library_host(directory: &tempfile::TempDir) -> ScanWorkerHost {
    let image = directory.path().join("ordinary_image");
    std::fs::write(&image, b"abc").unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image(
        [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ],
        3,
    )
    .unwrap();
    let budget = ScanWorkerRuntimeBudget::new(
        ProtocolLimits {
            max_frame_bytes: 64 << 10,
            max_stream_bytes: 1 << 20,
            max_nodes: 10,
            max_depth: 8,
        },
        64,
        1,
    )
    .unwrap();
    ScanWorkerHost::new(std::fs::File::open(image).unwrap(), expected, budget).unwrap()
}

fn managed_empty_host(directory: &tempfile::TempDir) -> CliEngineHost {
    let host = admitted_library_host(directory);
    let (engine, recovery) = Engine::open_with_scan_worker(
        EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        },
        host,
    )
    .unwrap();
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    let engine = Arc::new(engine);
    CliEngineHost {
        engine,
        recovery: Some(recovery),
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        slot: None,
        #[cfg(windows)]
        probe_recovery: diskgraph_engine::ProbeHost::new(1).unwrap().1,
    }
}

#[test]
fn late_startup_rejection_seals_original_registry() {
    let directory = tempfile::tempdir().unwrap();
    let session = managed_empty_host(&directory);
    let survivor = Arc::clone(&session.engine);
    assert!(directory.path().join("data").exists());
    let result = session.finish_open_until(std::time::Instant::now());
    assert!(matches!(
        result,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::BudgetExceeded
        ))
    ));
    assert_original_registry_sealed(&survivor, &directory);
}

fn assert_original_registry_sealed(survivor: &Arc<Engine>, directory: &tempfile::TempDir) {
    assert!(survivor.server_id().is_ok());
    let root = directory.path().join("scope");
    std::fs::create_dir(&root).unwrap();
    let principal = diskgraph_core::PrincipalId::new("retired-host-test").unwrap();
    survivor.bootstrap_local_admin(&principal).unwrap();
    let auth = survivor.policy_authorizer().unwrap();
    let scope = survivor.register_scope(&root, &principal, &auth).unwrap();
    let job = survivor
        .index_scope(&scope, &principal, &survivor.policy_authorizer().unwrap())
        .unwrap();
    let outcome = survivor.run_job_strict(&job.job_id, "retired-host");
    assert!(
        matches!(
            outcome,
            Err(EngineError::Business(
                diskgraph_core::BusinessError::Conflict
            ))
        ),
        "retired original registry must reject new births before image execution: {outcome:?}"
    );
    assert!(survivor.latest_revision(&scope).unwrap().is_none());
    // 没有 child 出生，只验证同一宿主异常边界，不能代替活跃 child 回收验收。
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn unconfirmed_user_domain_refuses_cli_before_any_database_birth() {
    use std::os::unix::fs::PermissionsExt;
    let domain_directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        domain_directory.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let domain = diskgraph_engine::TrustedLocalRecoveryDomain::from_host(
        std::fs::File::open(domain_directory.path()).unwrap(),
        until,
    )
    .unwrap();
    for _ in 0..4 {
        drop(domain.reserve(until).unwrap().activate(until).unwrap());
    }
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("not_born");
    // 只验证库已准入镜像后的 CLI 启动边界，不执行这个普通镜像或声称原生扫描资格。
    let result = CliEngineHost::open_admitted_until(
        EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        },
        Some(admitted_library_host(&directory)),
        until,
        || domain.reserve(until).map(Some).map_err(super::slot_error),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::RecoveryUnconfirmed
        ))
    ));
    assert!(
        !data.exists(),
        "unconfirmed capacity allowed CLI database birth"
    );
    for index in 0..4 {
        assert_eq!(
            std::fs::read(
                domain_directory
                    .path()
                    .join(format!("supervisor_{index}.slot"))
            )
            .unwrap(),
            b"DGSL01A\n"
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn admitted_cli_retires_original_engine_and_slot_after_command_error() {
    use std::os::unix::fs::PermissionsExt;
    let domain_directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        domain_directory.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let domain = diskgraph_engine::TrustedLocalRecoveryDomain::from_host(
        std::fs::File::open(domain_directory.path()).unwrap(),
        until,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let host = CliEngineHost::open_admitted_until(
        EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        },
        Some(admitted_library_host(&directory)),
        until,
        || domain.reserve(until).map(Some).map_err(super::slot_error),
    )
    .unwrap();
    let original = std::sync::Arc::downgrade(&host.engine);
    let result: Result<(), EngineError> =
        host.execute(|_| Err(diskgraph_core::BusinessError::PermissionDenied.into()));
    assert!(matches!(
        result,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::PermissionDenied
        ))
    ));
    assert!(
        original.upgrade().is_none(),
        "original Engine must retire before CLEAN"
    );
    assert_eq!(
        std::fs::read(domain_directory.path().join("supervisor_0.slot")).unwrap(),
        b"DGSL01C\n"
    );
    domain
        .reserve(until)
        .unwrap()
        .abort_before_birth(until)
        .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn occupied_user_domain_refuses_cli_before_database_birth() {
    use std::os::unix::fs::PermissionsExt;
    let domain_directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        domain_directory.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let domain = diskgraph_engine::TrustedLocalRecoveryDomain::from_host(
        std::fs::File::open(domain_directory.path()).unwrap(),
        until,
    )
    .unwrap();
    let original: Vec<_> = (0..4).map(|_| domain.reserve(until).unwrap()).collect();
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("not_born");
    let result = CliEngineHost::open_admitted_until(
        EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        },
        Some(admitted_library_host(&directory)),
        until,
        || domain.reserve(until).map(Some).map_err(super::slot_error),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::ResourceExhausted
        ))
    ));
    assert!(!data.exists());
    for slot in original {
        slot.abort_before_birth(until).unwrap();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn database_construction_failure_aborts_only_original_prebirth_reservation() {
    use std::os::unix::fs::PermissionsExt;
    let domain_directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        domain_directory.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let domain = diskgraph_engine::TrustedLocalRecoveryDomain::from_host(
        std::fs::File::open(domain_directory.path()).unwrap(),
        until,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("ordinary_file");
    std::fs::write(&data, b"keep original file").unwrap();
    let result = CliEngineHost::open_admitted_until(
        EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        },
        Some(admitted_library_host(&directory)),
        until,
        || domain.reserve(until).map(Some).map_err(super::slot_error),
    );
    assert!(matches!(result, Err(EngineError::Io(_))));
    assert_eq!(std::fs::read(data).unwrap(), b"keep original file");
    assert_eq!(
        std::fs::read(domain_directory.path().join("supervisor_0.slot")).unwrap(),
        b"DGSL01C\n"
    );
    domain
        .reserve(until)
        .unwrap()
        .abort_before_birth(until)
        .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn invalid_user_domain_requires_attention_before_database_birth() {
    use std::os::unix::fs::PermissionsExt;
    let capacity = tempfile::tempdir().unwrap();
    std::fs::set_permissions(capacity.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let record = capacity.path().join("supervisor_0.slot");
    std::fs::write(&record, b"DGSL99C\n").unwrap();
    std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o600)).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let domain = diskgraph_engine::TrustedLocalRecoveryDomain::from_host(
        std::fs::File::open(capacity.path()).unwrap(),
        until,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("not_born");
    let result = CliEngineHost::open_admitted_until(
        EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        },
        Some(admitted_library_host(&directory)),
        until,
        || domain.reserve(until).map(Some).map_err(super::slot_error),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::NeedsAttention
        ))
    ));
    assert!(!data.exists());
    assert_eq!(std::fs::read(record).unwrap(), b"DGSL99C\n");
}
