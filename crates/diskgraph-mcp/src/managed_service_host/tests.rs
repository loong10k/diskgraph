//! 实际 MCP 构造前准入，不执行普通镜像夹具。
use super::ManagedServiceHost;
use diskgraph_core::BusinessError;
use diskgraph_engine::{
    EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRuntimeBudget,
};
use diskgraph_mcp::McpConfig;
use std::time::{Duration, Instant};
fn host(directory: &tempfile::TempDir) -> ScanWorkerHost {
    let image = directory.path().join("image");
    std::fs::write(&image, b"abc").unwrap();
    let digest = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22,
        0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
        0x15, 0xad,
    ];
    ScanWorkerHost::new(
        std::fs::File::open(image).unwrap(),
        ScanWorkerHostConfig::from_expected_image(digest, 3).unwrap(),
        ScanWorkerRuntimeBudget::new(
            diskgraph_scan_worker::ProtocolLimits {
                max_frame_bytes: 64 << 10,
                max_stream_bytes: 1 << 20,
                max_nodes: 10,
                max_depth: 8,
            },
            64,
            1,
        )
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn capacity_refusal_precedes_local_and_remote_database_birth() {
    for trusted in [true, false] {
        for refusal in [
            BusinessError::ResourceExhausted,
            BusinessError::RecoveryUnconfirmed,
            BusinessError::NeedsAttention,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("not_born");
            let result = ManagedServiceHost::open_admitted(
                McpConfig {
                    data_dir: data.clone(),
                    ..McpConfig::default()
                },
                Some(host(&dir)),
                trusted,
                Instant::now() + Duration::from_secs(5),
                || Err(refusal.into()),
            );
            assert!(matches!(result,Err(EngineError::Business(error)) if error == refusal));
            assert!(!data.exists(), "MCP database born before capacity refusal");
        }
    }
}

fn domain(
    directory: &tempfile::TempDir,
    until: Instant,
) -> diskgraph_engine::TrustedLocalRecoveryDomain {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    diskgraph_engine::TrustedLocalRecoveryDomain::from_host(
        std::fs::File::open(directory.path()).unwrap(),
        until,
    )
    .unwrap()
}
#[test]
fn actual_active_slots_refuse_both_transport_modes_without_database_birth() {
    let capacity = tempfile::tempdir().unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    let domain = domain(&capacity, until);
    for _ in 0..4 {
        drop(domain.reserve(until).unwrap().activate(until).unwrap());
    }
    for trusted in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("not_born");
        let result = ManagedServiceHost::open_admitted(
            McpConfig {
                data_dir: data.clone(),
                ..McpConfig::default()
            },
            Some(host(&dir)),
            trusted,
            until,
            || domain.reserve(until).map(Some).map_err(EngineError::from),
        );
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::RecoveryUnconfirmed))
        ));
        assert!(!data.exists());
    }
    for index in 0..4 {
        assert_eq!(
            std::fs::read(capacity.path().join(format!("supervisor_{index}.slot"))).unwrap(),
            b"DGSL01A\n"
        );
    }
}
#[test]
fn actual_constructor_error_preserves_source_and_aborts_prebirth_slot() {
    let capacity = tempfile::tempdir().unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    let domain = domain(&capacity, until);
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("ordinary_file");
    std::fs::write(&data, b"unchanged").unwrap();
    let result = ManagedServiceHost::open_admitted(
        McpConfig {
            data_dir: data.clone(),
            ..McpConfig::default()
        },
        Some(host(&dir)),
        false,
        until,
        || domain.reserve(until).map(Some).map_err(EngineError::from),
    );
    assert!(matches!(result, Err(EngineError::Io(_))));
    assert_eq!(std::fs::read(data).unwrap(), b"unchanged");
    assert_eq!(
        std::fs::read(capacity.path().join("supervisor_0.slot")).unwrap(),
        b"DGSL01C\n"
    );
    domain
        .reserve(until)
        .unwrap()
        .abort_before_birth(until)
        .unwrap();
}
#[test]
fn original_service_clone_must_retire_before_clean() {
    let capacity = tempfile::tempdir().unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    let domain = domain(&capacity, until);
    let dir = tempfile::tempdir().unwrap();
    let admitted = ManagedServiceHost::open_admitted(
        McpConfig {
            data_dir: dir.path().join("data"),
            ..McpConfig::default()
        },
        Some(host(&dir)),
        false,
        until,
        || domain.reserve(until).map(Some).map_err(EngineError::from),
    )
    .unwrap();
    let survivor = admitted.service.clone();
    let parts = admitted
        .service
        .into_supervisor_parts(admitted.recovery, admitted.slot.unwrap());
    let weak = std::sync::Arc::downgrade(&parts.engine);
    let mut owner = match diskgraph_engine::SupervisorOwner::bind(parts, until) {
        Ok(owner) => owner,
        Err(_) => panic!("original binding refused"),
    };
    assert!(!owner.poll_retirement(until).unwrap());
    assert_eq!(
        std::fs::read(capacity.path().join("supervisor_0.slot")).unwrap(),
        b"DGSL01A\n"
    );
    drop(survivor);
    assert!(owner.poll_retirement(until).unwrap());
    assert!(weak.upgrade().is_none());
    assert_eq!(
        std::fs::read(capacity.path().join("supervisor_0.slot")).unwrap(),
        b"DGSL01C\n"
    );
}

#[test]
fn capacity_admission_preserves_local_and_remote_authority() {
    use diskgraph_core::{Authorizer, Decision, Permission};
    for trusted in [true, false] {
        let capacity = tempfile::tempdir().unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        let domain = domain(&capacity, until);
        let dir = tempfile::tempdir().unwrap();
        let config = McpConfig {
            data_dir: dir.path().join("data"),
            ..McpConfig::default()
        };
        let principal = config.principal.clone();
        let admitted =
            ManagedServiceHost::open_admitted(config, Some(host(&dir)), trusted, until, || {
                domain.reserve(until).map(Some).map_err(EngineError::from)
            })
            .unwrap();
        let decision = admitted
            .service
            .engine()
            .policy_authorizer()
            .unwrap()
            .decide(
                &principal,
                &Permission::ScopeAdmin,
                &diskgraph_engine::admin_scope(),
            );
        assert_eq!(matches!(decision, Decision::Allowed), trusted);
        crate::scan_worker_shutdown::finish_original(
            admitted
                .service
                .into_supervisor_parts(admitted.recovery, admitted.slot.unwrap()),
        );
        assert_eq!(
            std::fs::read(capacity.path().join("supervisor_0.slot")).unwrap(),
            b"DGSL01C\n"
        );
    }
}

#[test]
fn original_deadline_consumed_during_admission_refuses_unmanaged_database_birth() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("not_born");
    let until = Instant::now() + Duration::from_millis(10);
    let result = ManagedServiceHost::open_admitted(
        McpConfig {
            data_dir: data.clone(),
            ..McpConfig::default()
        },
        None,
        false,
        until,
        || {
            while Instant::now() < until {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(None)
        },
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert!(!data.exists());
}
