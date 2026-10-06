//! D42 逐资源占用原生验收；来源：真实 Linux scan epoch、procfs 和隔离文件持有者。
//! 必须在支持 tmpfs opaque handle / STATX_MNT_ID_UNIQUE 的 Linux lane 运行；失败不 skip。
#![cfg(target_os = "linux")]

#[path = "support/native_scan_engine.rs"]
mod native_scan_engine;
use native_scan_engine::NativeScanEngine;

use diskgraph_core::{
    IndexedFileEpoch, PrincipalId, ProcessEvidenceFailureCode, ProcessEvidenceLimits,
    ProcessObservationCoverage, QueryBudget, QueryReadBudget,
};
use diskgraph_engine::{EngineConfig, native_process::ProcessNativeSession};
use diskgraph_store::SqliteSnapshotStore;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// 原生普通文件及真实持久扫描身份；来源：公开 Engine API，无伪造 epoch 或 SQL。
struct ObservationFixture {
    _source: tempfile::TempDir,
    _data: tempfile::TempDir,
    root: PathBuf,
    file_epoch: IndexedFileEpoch,
    other_epoch: IndexedFileEpoch,
}
impl ObservationFixture {
    fn new() -> Self {
        let source =
            tempfile::tempdir_in("/dev/shm").expect("Linux native acceptance requires tmpfs");
        let data = tempfile::tempdir().unwrap();
        let root = source.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("held"),
            b"fixture secret body must not enter summary",
        )
        .unwrap();
        std::fs::write(root.join("other"), b"unrelated").unwrap();
        let engine = NativeScanEngine::open(EngineConfig {
            data_dir: data.path().to_owned(),
            ..EngineConfig::default()
        })
        .unwrap();
        let actor = PrincipalId::new("native-process-fixture").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let auth = engine.policy_authorizer().unwrap();
        let scope = engine.register_scope(&root, &actor, &auth).unwrap();
        let auth = engine.policy_authorizer().unwrap();
        let job = engine.index_scope(&scope, &actor, &auth).unwrap();
        engine.run_job(&job.job_id, "native-process-scan").unwrap();
        let revision = engine.revision_for_job(&job.job_id, &actor, &auth).unwrap();
        let snapshot = engine.revision_snapshot(&revision).unwrap();
        let store = SqliteSnapshotStore::open(&data.path().join("diskgraph.sqlite")).unwrap();
        let epoch = |path: &str| {
            let node = engine
                .revision_node_at(&revision, Path::new(path))
                .unwrap()
                .unwrap();
            let mut budget = QueryReadBudget::new(
                QueryBudget::default(),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
            let saved = store
                .unix_observation_bounded(&snapshot.id, node.id, &mut budget)
                .unwrap()
                .unwrap();
            assert!(
                saved.gap.is_none(),
                "native environment/implementation unverified: {:?}",
                saved.gap
            );
            saved.observation.unwrap().epoch().clone()
        };
        let file_epoch = epoch("held");
        let other_epoch = epoch("other");
        Self {
            _source: source,
            _data: data,
            root,
            file_epoch,
            other_epoch,
        }
    }
}

#[test]
fn native_observation_excludes_only_its_descriptor_and_keeps_real_same_pid_and_child() {
    let fixture = ObservationFixture::new();
    let limits = ProcessEvidenceLimits::default();
    let cancel = AtomicBool::new(false);
    let session = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &|| Ok(())).unwrap();
    let absent = session
        .observe_linux_file(&fixture.root, Path::new("held"), &fixture.file_epoch)
        .unwrap();
    assert!(
        !absent
            .processes()
            .iter()
            .any(|p| p.pid() == std::process::id()),
        "observer target FD must not create its own positive"
    );
    assert_eq!(absent.coverage(), ProcessObservationCoverage::Partial);
    let own = File::open(fixture.root.join("held")).unwrap();
    let mut child = Command::new("/bin/sh")
        .args(["-c", "exec 3< \"$1\"; printf ready; read ignored", "holder"])
        .arg(fixture.root.join("held"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = [0_u8; 5];
    child.stdout.take().unwrap().read_exact(&mut ready).unwrap();
    assert_eq!(&ready, b"ready");
    let sample = session
        .observe_linux_file(&fixture.root, Path::new("held"), &fixture.file_epoch)
        .unwrap();
    assert!(
        sample
            .processes()
            .iter()
            .any(|p| p.pid() == std::process::id())
    );
    assert!(sample.processes().iter().any(|p| p.pid() == child.id()));
    let other = session
        .observe_linux_file(&fixture.root, Path::new("other"), &fixture.other_epoch)
        .unwrap();
    assert!(
        !other
            .processes()
            .iter()
            .any(|p| p.pid() == std::process::id() || p.pid() == child.id()),
        "resource union must not fabricate an unrelated edge"
    );
    assert_eq!(
        sample.visibility_domain_sha256(),
        other.visibility_domain_sha256()
    );
    let text = serde_json::to_string(&sample).unwrap();
    assert!(!text.contains("fixture secret"));
    assert!(!text.contains("/dev/shm"));
    assert!(!text.contains("argv"));
    assert!(!text.contains("holder"));
    drop(child.stdin.take());
    child.wait().unwrap();
    drop(own);
}

#[test]
fn replacement_rejects_the_original_indexed_epoch_and_failure_is_sticky() {
    let fixture = ObservationFixture::new();
    std::fs::rename(
        fixture.root.join("held"),
        fixture.root.join("retained-original"),
    )
    .unwrap();
    std::fs::write(fixture.root.join("held"), b"replacement").unwrap();
    let limits = ProcessEvidenceLimits::default();
    let cancel = AtomicBool::new(false);
    let session = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &|| Ok(())).unwrap();
    assert_eq!(
        session.observe_linux_file(&fixture.root, Path::new("held"), &fixture.file_epoch),
        Err(ProcessEvidenceFailureCode::Conflict)
    );
    let spent = session.usage();
    assert_eq!(
        session.observe_linux_file(&fixture.root, Path::new("other"), &fixture.other_epoch),
        Err(ProcessEvidenceFailureCode::Conflict)
    );
    assert_eq!(session.usage(), spent);
}
