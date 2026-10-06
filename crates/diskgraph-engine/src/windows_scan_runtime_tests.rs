//! Windows 实际 Runtime 与恢复租约测试；必须使用当前 Cargo 构建的真实扫描器。
use crate::scan_worker_runtime::ScanWorkerRuntime;
use crate::{EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRuntimeBudget};
use diskgraph_core::BusinessError;
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::ProtocolLimits;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};

fn host() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    ScanWorkerHost,
    Instant,
) {
    let source = std::env::var_os("DISKGRAPH_WINDOWS_WORKER_QUALIFICATION")
        .expect("requires actual current Cargo worker");
    let bytes = std::fs::read(source).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("qualified-worker.exe");
    std::fs::write(&image, &bytes).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let host = ScanWorkerHost::new_until(
        File::open(&image).unwrap(),
        ScanWorkerHostConfig::from_expected_image(
            Sha256::digest(&bytes).into(),
            bytes.len() as u64,
        )
        .unwrap(),
        ScanWorkerRuntimeBudget::new(
            ProtocolLimits {
                max_frame_bytes: 65536,
                max_stream_bytes: 1048576,
                max_nodes: 4096,
                max_depth: 64,
            },
            4096,
            1,
        )
        .unwrap(),
        deadline,
        &mut || Ok(()),
    )
    .unwrap();
    (directory, image, host, deadline)
}

#[test]
fn actual_bound_runtime_scan_returns_complete_tree() {
    let (_image_directory, _image, host, deadline) = host();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("first"), [1; 17]).unwrap();
    std::fs::write(root.path().join("second"), [2; 23]).unwrap();
    let tree = ScanWorkerRuntime::new(&host, deadline)
        .run(
            root.path(),
            &ScanOptions {
                apparent_size: true,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: Some(8),
                dedup_hardlinks: true,
                metric: diskgraph_disktree_core::tree::Metric::Bytes,
            },
            &mut || Ok(()),
            &mut || Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();
    let root = tree;
    assert_eq!(root.files, 2);
    assert_eq!(root.bytes, 40);
    assert_eq!(root.children.len(), 2);
    assert!(
        host.registry.drain().unwrap(),
        "no active or retained worker after normal completion"
    );
    eprintln!("DG_WINDOWS_BOUND_RUNTIME_COMPLETE_SCAN=1");
}

#[test]
fn suspended_binding_rejection_retains_image_after_host_release() {
    use crate::native_child::{ChildInputMode, ChildSpawnError, WindowsChild};
    let (_directory, image, host, deadline) = host();
    let lease = host
        .prepare_windows_image(deadline, &mut || Ok(()))
        .unwrap();
    let weak = std::sync::Arc::downgrade(&lease);
    let mut command = std::process::Command::new(&image);
    command.env_clear().current_dir(image.parent().unwrap());
    let mut owner = None;
    let mut phases = 0;
    let result = WindowsChild::spawn_into_with_binding(
        &mut command,
        ChildInputMode::WorkerControl,
        &mut owner,
        || lease.validate(deadline, &mut || Ok(())),
        || {
            phases += 1;
            Ok(())
        },
        Some(std::sync::Arc::clone(&lease)),
        |process| {
            lease.verify_process_name(process, deadline, &mut || Ok(()))?;
            Err::<(), EngineError>(BusinessError::PermissionDenied.into())
        },
    );
    assert!(matches!(
        result,
        Err(ChildSpawnError::Checkpoint {
            primary: EngineError::Business(BusinessError::PermissionDenied),
            cleanup: None,
        })
    ));
    assert_eq!(
        phases, 3,
        "original child must remain suspended before fourth phase"
    );
    drop(lease);
    drop(host);
    let retained = owner
        .as_mut()
        .expect("original suspended child retains image");
    assert!(weak.upgrade().is_some());
    assert_eq!(
        OpenOptions::new()
            .write(true)
            .open(&image)
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
    retained.cleanup().unwrap();
    drop(owner);
    assert!(weak.upgrade().is_none());
    OpenOptions::new().write(true).open(&image).unwrap();
    eprintln!("DG_WINDOWS_FAILED_OWNER_RETAINS_IMAGE_AFTER_HOST_DROP=1");
}

#[test]
fn actual_bound_runtime_preserves_postbirth_revocation() {
    let (_directory, _image, host, deadline) = host();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("first"), [1; 17]).unwrap();
    let revoked = std::cell::Cell::new(false);
    let result = ScanWorkerRuntime::new(&host, deadline).run(
        root.path(),
        &ScanOptions::default(),
        &mut || {
            if revoked.get() {
                Err(BusinessError::PermissionDenied.into())
            } else {
                Ok(())
            }
        },
        &mut || {
            revoked.set(true);
            Ok(())
        },
        &mut |_| Ok(()),
    );
    assert!(revoked.get(), "must reach actual postbirth driver polling");
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert!(
        host.registry.drain().unwrap(),
        "original cancelled worker must be actually disposed"
    );
    eprintln!("DG_WINDOWS_BOUND_RUNTIME_ORIGINAL_REVOCATION=1");
}
