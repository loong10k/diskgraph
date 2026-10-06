//! 原挂起进程名称与宿主镜像材料绑定；不把名称匹配当映射字节或执行许可。
use crate::native_child::{ChildInputMode, ChildSpawnError, WindowsTestBirth};
use crate::{EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRuntimeBudget};
use diskgraph_core::BusinessError;
use diskgraph_scan_worker::ProtocolLimits;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::os::windows::io::BorrowedHandle;
use std::process::Command;
use std::time::{Duration, Instant};

fn inspect(foreign: bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let original = std::env::current_exe().unwrap();
    let bytes = std::fs::read(&original).unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image(
        Sha256::digest(&bytes).into(),
        bytes.len() as u64,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let selected = if foreign {
        let other = directory.path().join("same-bytes.exe");
        std::fs::write(&other, &bytes).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), bytes);
        other
    } else {
        original.clone()
    };
    let host = ScanWorkerHost::new_until(
        File::open(selected).unwrap(),
        expected,
        ScanWorkerRuntimeBudget::new(
            ProtocolLimits {
                max_frame_bytes: 65536,
                max_stream_bytes: 1048576,
                max_nodes: 10,
                max_depth: 4,
            },
            4096,
            1,
        )
        .unwrap(),
        deadline,
        &mut || Ok(()),
    )
    .unwrap();
    let mut owner = None;
    let mut checks = 0;
    let mut command = Command::new(original);
    command.env_clear();
    let birth = WindowsTestBirth::spawn_into(
        &mut command,
        ChildInputMode::WorkerControl,
        &mut owner,
        || {
            checks += 1;
            if checks == 3 {
                Err("inspect actual suspended image name")
            } else {
                Ok(())
            }
        },
    );
    let child = owner
        .as_mut()
        .expect("actual suspended child retained outside birth");
    // 该原进程句柄在全部同步核验及实际cleanup期间均由child唯一持有。
    let process = unsafe { BorrowedHandle::borrow_raw(child.process.as_ref().unwrap().as_raw()) };
    let result = host.verify_windows_process_image_name(process, deadline, &mut || Ok(()));
    let mut expired_checks = 0;
    let expired = host.verify_windows_process_image_name(
        process,
        Instant::now() - Duration::from_secs(1),
        &mut || {
            expired_checks += 1;
            Ok(())
        },
    );
    let denied = host.verify_windows_process_image_name(process, deadline, &mut || {
        Err(BusinessError::PermissionDenied.into())
    });
    let cleanup = child.cleanup();
    assert!(matches!(
        birth,
        Err(ChildSpawnError::Checkpoint {
            primary: "inspect actual suspended image name",
            cleanup: None
        })
    ));
    assert_eq!(checks, 3);
    cleanup.unwrap();
    assert_eq!(expired_checks, 1);
    assert!(matches!(
        expired,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert!(matches!(
        denied,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    if foreign {
        assert!(
            matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
            "equal bytes at another name must not bind: {result:?}"
        );
        eprintln!("DG_WINDOWS_PROCESS_FOREIGN_NAME_REJECTED=1");
    } else {
        result.unwrap();
        eprintln!("DG_WINDOWS_PROCESS_ORIGINAL_NAME_BOUND=1");
    }
}

#[test]
fn actual_suspended_image_name_matches_original_material() {
    inspect(false);
}

#[test]
fn actual_suspended_image_name_rejects_byte_identical_foreign_material() {
    inspect(true);
}
