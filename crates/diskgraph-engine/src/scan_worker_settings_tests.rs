//! 部署配置的信任边界回归；来源：PF-06 本地宿主配置，非执行资格测试。
use crate::scan_worker_settings::ScanWorkerSettings;
use crate::{EngineError, ScanWorkerInstallation};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::time::{Duration, Instant};

fn settings(values: [Option<OsString>; 3]) -> Result<Option<ScanWorkerSettings>, EngineError> {
    ScanWorkerSettings::from_lookup(|name| match name {
        "DISKGRAPH_SCAN_WORKER_PATH" => values[0].clone(),
        "DISKGRAPH_SCAN_WORKER_SHA256" => values[1].clone(),
        "DISKGRAPH_SCAN_WORKER_BYTES" => values[2].clone(),
        _ => panic!("unexpected environment key"),
    })
}

fn runtime() -> crate::ScanWorkerRuntimeBudget {
    crate::ScanWorkerRuntimeBudget::new(
        diskgraph_scan_worker::ProtocolLimits {
            max_frame_bytes: 64 << 10,
            max_stream_bytes: 8 << 20,
            max_nodes: 1000,
            max_depth: 32,
        },
        0,
        1,
    )
    .unwrap()
}

#[test]
fn unified_host_preserves_original_cancel_before_environment_or_paths() {
    assert!(matches!(
        ScanWorkerSettings::host_from_environment(
            runtime(),
            Instant::now() - Duration::from_secs(1),
            &mut || Err(EngineError::Poisoned),
        ),
        Err(EngineError::Poisoned)
    ));
}

#[test]
fn unified_host_rejects_original_expired_deadline_before_configuration() {
    assert!(matches!(
        ScanWorkerSettings::host_from_environment(
            runtime(),
            Instant::now() - Duration::from_secs(1),
            &mut || Ok(()),
        ),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn unified_host_partial_configuration_stays_invalid_and_never_opens_image() {
    let result = ScanWorkerSettings::host_from_lookup(
        runtime(),
        Instant::now() + Duration::from_secs(2),
        &mut || Ok(()),
        |name| (name == "DISKGRAPH_SCAN_WORKER_PATH").then(|| OsString::from("/missing-helper")),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));
}

#[test]
fn unified_host_absent_configuration_still_rechecks_original_cancel() {
    let mut calls = 0;
    let result = ScanWorkerSettings::host_from_lookup(
        runtime(),
        Instant::now() + Duration::from_secs(2),
        &mut || {
            calls += 1;
            if calls > 1 {
                Err(EngineError::Poisoned)
            } else {
                Ok(())
            }
        },
        |_| None,
    );
    assert!(matches!(result, Err(EngineError::Poisoned)));
    assert_eq!(calls, 2);
}

#[cfg(target_os = "macos")]
#[test]
fn unified_macos_host_refuses_complete_legacy_environment_without_opening_file() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("must-never-open");
    let result = ScanWorkerSettings::host_from_lookup(
        runtime(),
        Instant::now() + Duration::from_secs(2),
        &mut || Ok(()),
        |name| match name {
            "DISKGRAPH_SCAN_WORKER_PATH" => Some(missing.clone().into_os_string()),
            "DISKGRAPH_SCAN_WORKER_SHA256" => Some("a".repeat(64).into()),
            "DISKGRAPH_SCAN_WORKER_BYTES" => Some("1".into()),
            _ => panic!("unexpected lookup"),
        },
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Unsupported))
    ));
    assert!(!missing.exists());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unified_nonmacos_host_preserves_unconfigured_none() {
    assert!(
        ScanWorkerSettings::host_from_lookup(
            runtime(),
            Instant::now() + Duration::from_secs(2),
            &mut || Ok(()),
            |_| None,
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn unified_host_invalid_digest_preserves_argument_error() {
    let directory = tempfile::tempdir().unwrap();
    let result = ScanWorkerSettings::host_from_lookup(
        runtime(),
        Instant::now() + Duration::from_secs(2),
        &mut || Ok(()),
        |name| match name {
            "DISKGRAPH_SCAN_WORKER_PATH" => Some(directory.path().join("absent").into_os_string()),
            "DISKGRAPH_SCAN_WORKER_SHA256" => Some("z".repeat(64).into()),
            "DISKGRAPH_SCAN_WORKER_BYTES" => Some("1".into()),
            _ => panic!("unexpected lookup"),
        },
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));
}

#[test]
fn unified_host_never_resets_deadline_after_lookup() {
    let expired = std::cell::Cell::new(false);
    let original = Instant::now() + Duration::from_millis(10);
    let result = ScanWorkerSettings::host_from_lookup(runtime(), original, &mut || Ok(()), |_| {
        if !expired.replace(true) {
            std::thread::sleep(
                original.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            );
        }
        None
    });
    assert!(
        expired.get(),
        "lookup must actually cross the original deadline"
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn unified_macos_absent_environment_uses_fixed_installation_semantics() {
    // 只读真实固定位置；无配置必须为None，存在/损坏必须与真实Engine准入一致，不能假定机器未安装。
    let deadline = Instant::now() + Duration::from_secs(10);
    let direct = crate::ScanWorkerHost::from_installed_macos(runtime(), deadline, &mut || Ok(()));
    let unified =
        ScanWorkerSettings::host_from_lookup(runtime(), deadline, &mut || Ok(()), |_| None);
    match (direct, unified) {
        (Ok(left), Ok(right)) => assert_eq!(left.is_some(), right.is_some()),
        (Err(left), Err(right)) => assert_eq!(format!("{left:?}"), format!("{right:?}")),
        _ => panic!("absent legacy environment must delegate to the fixed protected installation"),
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unified_nonmacos_complete_configuration_opens_original_local_image() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("worker");
    std::fs::write(&path, b"x").unwrap();
    let result = ScanWorkerSettings::host_from_lookup(
        runtime(),
        Instant::now() + Duration::from_secs(2),
        &mut || Ok(()),
        |name| match name {
            "DISKGRAPH_SCAN_WORKER_PATH" => Some(path.clone().into_os_string()),
            "DISKGRAPH_SCAN_WORKER_SHA256" => Some(format!("{:x}", Sha256::digest(b"x")).into()),
            "DISKGRAPH_SCAN_WORKER_BYTES" => Some("1".into()),
            _ => panic!("unexpected lookup"),
        },
    );
    assert!(result.unwrap().is_some());
}

#[test]
fn absent_configuration_is_distinct_from_every_partial_configuration() {
    assert!(settings([None, None, None]).unwrap().is_none());
    let complete = [
        OsString::from("/worker"),
        OsString::from("a".repeat(64)),
        OsString::from("1"),
    ];
    for mask in 1..7 {
        let input = std::array::from_fn(|index| {
            (mask & (1 << index) != 0).then(|| complete[index].clone())
        });
        assert!(matches!(
            settings(input),
            Err(EngineError::Business(BusinessError::InvalidArgument))
        ));
    }
}

#[test]
fn invalid_material_is_rejected_before_opening_any_image() {
    let directory = tempfile::tempdir().unwrap();
    let absolute = directory.path().join("worker").into_os_string();
    for hash in [
        "a".repeat(63),
        "g".repeat(64),
        format!(" {}", "a".repeat(64)),
    ] {
        assert!(matches!(
            settings([Some(absolute.clone()), Some(hash.into()), Some("1".into())]),
            Err(EngineError::Business(BusinessError::InvalidArgument))
        ));
    }
    for length in ["", "0", "+1", " 1", "18446744073709551616"] {
        assert!(matches!(
            settings([
                Some(absolute.clone()),
                Some("a".repeat(64).into()),
                Some(length.into())
            ]),
            Err(EngineError::Business(BusinessError::InvalidArgument))
        ));
    }
    assert!(matches!(
        settings([
            Some("relative-worker".into()),
            Some("a".repeat(64).into()),
            Some("1".into())
        ]),
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));
    assert!(matches!(
        settings([
            Some(absolute),
            Some("a".repeat(64).into()),
            Some("134217729".into())
        ]),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn independent_expected_digest_and_original_open_file_survive_name_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("worker");
    let trusted = b"trusted installation material";
    std::fs::write(&path, trusted).unwrap();
    let hash = format!("{:x}", Sha256::digest(trusted));
    let config = settings([
        Some(path.clone().into_os_string()),
        Some(hash.into()),
        Some(trusted.len().to_string().into()),
    ])
    .unwrap()
    .unwrap();
    // 可写邻接清单不参与建立预期值；打开句柄后原名称换成另一内容。
    std::fs::write(
        directory.path().join("scan-worker-manifest.json"),
        b"untrusted",
    )
    .unwrap();
    let (file, expected) = config.open_held().unwrap();
    std::fs::rename(&path, directory.path().join("original-worker")).unwrap();
    std::fs::write(&path, b"different contents").unwrap();
    let verified = ScanWorkerInstallation::verify(
        file,
        &expected,
        Instant::now() + Duration::from_secs(2),
        || Ok(()),
    );
    assert!(verified.is_ok(), "held material verification: {verified:?}");
}

#[test]
fn opening_missing_material_preserves_the_actual_io_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing-worker");
    let direct = std::fs::File::open(&path).unwrap_err();
    let config = settings([
        Some(path.into_os_string()),
        Some("a".repeat(64).into()),
        Some("1".into()),
    ])
    .unwrap()
    .unwrap();
    match config.open_held() {
        Err(EngineError::Io(error)) => {
            assert_eq!(error.kind(), direct.kind());
            assert_eq!(error.raw_os_error(), direct.raw_os_error());
        }
        other => panic!("actual missing image error: {other:?}"),
    }
}

#[test]
fn neighboring_correct_values_cannot_override_wrong_but_well_formed_expectations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("worker");
    let trusted = b"independent deployment expected values";
    std::fs::write(&path, trusted).unwrap();
    let correct = format!("{:x}", Sha256::digest(trusted));
    std::fs::write(directory.path().join("scan-worker-manifest.json"), serde_json::to_vec(&serde_json::json!({
        "schema_version": 1, "executable": {"name": "worker", "sha256": correct, "bytes": trusted.len()}
    })).unwrap()).unwrap();
    for (digest, length) in [
        ("00".repeat(32), trusted.len()),
        (correct, trusted.len() + 1),
    ] {
        let config = settings([
            Some(path.clone().into_os_string()),
            Some(digest.into()),
            Some(length.to_string().into()),
        ])
        .unwrap()
        .unwrap();
        let (file, expected) = config.open_held().unwrap();
        let result = ScanWorkerInstallation::verify(
            file,
            &expected,
            Instant::now() + Duration::from_secs(2),
            || Ok(()),
        );
        assert!(
            matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
            "independent expected values: {result:?}"
        );
    }
}

#[test]
fn uppercase_complete_digest_is_preserved_as_the_same_expected_material() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("worker");
    std::fs::write(&path, b"x").unwrap();
    let config = settings([
        Some(path.into_os_string()),
        Some(format!("{:X}", Sha256::digest(b"x")).into()),
        Some("1".into()),
    ])
    .unwrap()
    .unwrap();
    let (file, expected) = config.open_held().unwrap();
    assert!(
        ScanWorkerInstallation::verify(
            file,
            &expected,
            Instant::now() + Duration::from_secs(2),
            || Ok(())
        )
        .is_ok()
    );
}

#[cfg(windows)]
#[test]
fn windows_drive_relative_root_relative_and_non_unicode_configuration_are_rejected() {
    use std::os::windows::ffi::OsStringExt;
    for path in ["C:worker", "\\worker"] {
        assert!(matches!(
            settings([
                Some(path.into()),
                Some("a".repeat(64).into()),
                Some("1".into())
            ]),
            Err(EngineError::Business(BusinessError::InvalidArgument))
        ));
    }
    let directory = tempfile::tempdir().unwrap();
    let absolute = directory.path().join("worker").into_os_string();
    for input in [
        [
            Some(absolute.clone()),
            Some(OsString::from_wide(&[0xd800; 64])),
            Some("1".into()),
        ],
        [
            Some(absolute),
            Some("a".repeat(64).into()),
            Some(OsString::from_wide(&[0xd800])),
        ],
    ] {
        assert!(matches!(
            settings(input),
            Err(EngineError::Business(BusinessError::InvalidArgument))
        ));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn native_non_utf8_image_path_is_retained_but_non_utf8_digest_is_rejected() {
    use std::os::unix::ffi::OsStringExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(OsString::from_vec(vec![b'w', 0xff]));
    std::fs::write(&path, b"x").unwrap();
    let config = settings([
        Some(path.into_os_string()),
        Some(format!("{:x}", Sha256::digest(b"x")).into()),
        Some("1".into()),
    ])
    .unwrap()
    .unwrap();
    assert!(config.open_held().is_ok());
    assert!(matches!(
        settings([
            Some("/worker".into()),
            Some(OsString::from_vec(vec![0xff; 64])),
            Some("1".into())
        ]),
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));
}

#[cfg(unix)]
#[test]
fn native_non_utf8_open_failure_and_non_unicode_material_preserve_their_boundaries() {
    use std::os::unix::ffi::OsStringExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(OsString::from_vec(vec![b'w', 0xff]));
    let direct = std::fs::File::open(&path).unwrap_err();
    let config = settings([
        Some(path.into_os_string()),
        Some("a".repeat(64).into()),
        Some("1".into()),
    ])
    .unwrap()
    .unwrap();
    match config.open_held() {
        Err(EngineError::Io(error)) => {
            assert_eq!(error.kind(), direct.kind());
            assert_eq!(error.raw_os_error(), direct.raw_os_error());
        }
        other => panic!("native non-UTF8 open failure: {other:?}"),
    }
    for input in [
        [
            Some(directory.path().join("worker").into_os_string()),
            Some(OsString::from_vec(vec![0xff; 64])),
            Some("1".into()),
        ],
        [
            Some(directory.path().join("worker").into_os_string()),
            Some("a".repeat(64).into()),
            Some(OsString::from_vec(vec![0xff])),
        ],
    ] {
        assert!(matches!(
            settings(input),
            Err(EngineError::Business(BusinessError::InvalidArgument))
        ));
    }
}
