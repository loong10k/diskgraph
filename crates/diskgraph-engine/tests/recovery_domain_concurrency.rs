//! 原持久域稳定时并发出生前认领回归；不能代替实际 CLI/MCP 监督和 Intel 平台验收。
#![cfg(any(target_os = "linux", target_os = "macos"))]
use diskgraph_engine::TrustedLocalRecoveryDomain;
use diskgraph_engine::recovery_slot::SlotError;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CLAIMS: usize = 128;
const WORKERS: usize = 4;
const MARKER: &str = "DG_RECOVERY_DOMAIN_CONCURRENCY_ROOT";

/// 隔离子进程的原 owner；异常也必须终止并实际 wait，避免测试遗留认领者。
struct Worker {
    child: Child,
    output: File,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "由父测试在隔离子进程中明确运行，不计作独立平台通过"]
fn recovery_domain_worker() {
    let root = std::env::var_os(MARKER).expect("private test root required");
    let until = Instant::now() + Duration::from_secs(25);
    let domain = TrustedLocalRecoveryDomain::from_host(File::open(root).unwrap(), until).unwrap();
    for _ in 0..CLAIMS {
        loop {
            assert!(Instant::now() < until, "original worker deadline expired");
            match domain.reserve(until) {
                Ok(slot) => {
                    slot.abort_before_birth(until).unwrap();
                    break;
                }
                Err(SlotError::Busy) => std::thread::sleep(Duration::from_millis(1)),
                Err(error) => panic!("stable original domain admission failed: {error:?}"),
            }
        }
    }
    println!("DG_ORIGINAL_DOMAIN_CLAIMS_COMPLETE={CLAIMS}");
}

#[test]
fn parallel_prebirth_claims_keep_the_original_domain_and_clean_records() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let before = root.path().metadata().unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    let mut workers: Vec<Worker> = Vec::new();
    for _ in 0..WORKERS {
        let output = tempfile::tempfile().unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "recovery_domain_worker",
                "--nocapture",
            ])
            .env(MARKER, root.path())
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output.try_clone().unwrap())
            .spawn()
            .unwrap();
        workers.push(Worker { child, output });
    }
    for worker in &mut workers {
        let status = loop {
            if let Some(status) = worker.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < until,
                "original parent observation deadline expired"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        worker.output.seek(SeekFrom::Start(0)).unwrap();
        let mut output = String::new();
        (&mut worker.output)
            .take(16384)
            .read_to_string(&mut output)
            .unwrap();
        assert!(status.success(), "original domain child failed: {output}");
        assert!(
            output.contains(&format!("DG_ORIGINAL_DOMAIN_CLAIMS_COMPLETE={CLAIMS}")),
            "original claims did not all execute: {output}"
        );
        assert!(
            output.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"),
            "child exact test count changed: {output}"
        );
    }
    let after = root.path().metadata().unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    let entries: Vec<_> = std::fs::read_dir(root.path())
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!entries.is_empty() && entries.len() <= 4);
    for entry in entries {
        let name = entry.file_name();
        assert!((0..4).any(|index| name == format!("supervisor_{index}.slot").as_str()));
        let metadata = std::fs::symlink_metadata(entry.path()).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(std::fs::read(entry.path()).unwrap(), b"DGSL01C\n");
    }
    println!("DG_PARALLEL_ORIGINAL_DOMAIN_CLAIMS={}", CLAIMS * WORKERS);
}
