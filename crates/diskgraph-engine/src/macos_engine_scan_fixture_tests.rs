//! 显式候选feature下的真实macOS安装→Engine任务→发布验收，默认忽略且不得缺环境假PASS。
use crate::{
    Engine, EngineConfig, ScanWorkerHost, ScanWorkerRecovery, ScanWorkerRuntimeBudget, admin_scope,
};
use crate::{EngineError, scan_worker_runtime_hooks};
use diskgraph_core::{Authorizer, Decision, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
use diskgraph_scan_worker::ProtocolLimits;
use diskgraph_store::JobRecord;
use diskgraph_store::JobState;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const EXHAUSTED_RESPONSE_BYTES: u64 = 512;

fn fixture_root(directory: &Path) -> std::path::PathBuf {
    let root = directory.join("scope");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(root.join("nested")).unwrap();
    std::fs::write(root.join("one"), b"abc").unwrap();
    std::fs::write(root.join("nested/two"), b"abcde").unwrap();
    root
}

/// 隔离控制库/图库及普通目录的真实安装宿主；来源：PF-06，非Java迁移对象。
struct MacosEngineScanFixture {
    engine: Engine,
    recovery: ScanWorkerRecovery,
    actor: PrincipalId,
    policy: PolicyAuthorizer,
    scope: ScopeId,
    job: JobRecord,
    _directory: tempfile::TempDir,
}

impl MacosEngineScanFixture {
    fn new(response_bytes: u64) -> Self {
        assert_ne!(unsafe { libc::getuid() }, 0);
        assert_eq!(unsafe { libc::getuid() }, unsafe { libc::geteuid() });
        assert_eq!(required("GITHUB_ACTIONS"), "true");
        assert_eq!(required("RUNNER_OS"), "macOS");
        assert_eq!(required("RUNNER_ENVIRONMENT"), "github-hosted");
        let run = required("GITHUB_RUN_ID");
        assert!(!run.is_empty() && run.bytes().all(|byte| byte.is_ascii_digit()));
        assert_eq!(required("DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE"), run);
        // 本模块只在真实出生分支启用时编译；静态核对接线，不能把 Unsupported 当预算拒绝。
        const { assert!(cfg!(feature = "macos_native_scan_candidate")) };
        let deadline = Instant::now() + Duration::from_secs(60);
        let host = ScanWorkerHost::from_installed_macos(
            ScanWorkerRuntimeBudget::new(
                ProtocolLimits {
                    max_frame_bytes: 64 << 10,
                    max_stream_bytes: response_bytes,
                    max_nodes: 1000,
                    max_depth: 32,
                },
                0,
                1,
            )
            .unwrap(),
            deadline,
            &mut || Ok(()),
        )
        .unwrap()
        .expect("root fixture must install the candidate before ordinary Engine tests");
        let guard = crate::macos_installation_lock::MacosInstallationLock::acquire_shared(
            deadline,
            &mut || Ok(()),
        )
        .unwrap();
        let settings = crate::macos_host_settings::MacosHostSettings::read_authorized(
            &guard,
            deadline,
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(settings.active_epoch, 2);
        assert_eq!(
            settings.expected_bytes,
            required("DISKGRAPH_MACOS_FIXTURE_BYTES")
                .parse::<u64>()
                .unwrap()
        );
        let expected_hex = required("DISKGRAPH_MACOS_FIXTURE_SHA256");
        assert_eq!(expected_hex.len(), 64);
        assert!(expected_hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
        for (index, byte) in settings.expected_sha256.iter().enumerate() {
            assert_eq!(
                *byte,
                u8::from_str_radix(&expected_hex[index * 2..index * 2 + 2], 16).unwrap()
            );
        }
        drop(guard);
        let directory = tempfile::tempdir().unwrap();
        let root = fixture_root(directory.path());
        let (engine, recovery) = Engine::open_with_scan_worker(
            EngineConfig {
                data_dir: directory.path().join("data"),
                scan_options: diskgraph_disktree_core::scan::ScanOptions {
                    apparent_size: true,
                    ..diskgraph_disktree_core::scan::ScanOptions::default()
                },
                ..EngineConfig::default()
            },
            host,
        )
        .unwrap();
        let actor = PrincipalId::new("macos-actual-engine-candidate").unwrap();
        let mut policy = PolicyAuthorizer::new(1);
        policy.grant(actor.clone(), Permission::ScopeAdmin, admin_scope());
        engine.bootstrap_local_admin(&actor).unwrap();
        assert_eq!(
            engine.policy_authorizer().unwrap().decide(
                &actor,
                &Permission::ScopeAdmin,
                &admin_scope()
            ),
            Decision::Allowed
        );
        let scope = engine.register_scope(&root, &actor, &policy).unwrap();
        for permission in [
            Permission::IndexWrite,
            Permission::MetadataRead,
            Permission::OperationView,
        ] {
            policy.grant(actor.clone(), permission, scope.clone());
        }
        assert_eq!(
            engine
                .control_store()
                .unwrap()
                .live_permission(&actor, &Permission::IndexWrite, &scope)
                .unwrap(),
            Some(true)
        );
        let job = engine.index_scope(&scope, &actor, &policy).unwrap();
        assert_eq!(job.state, JobState::Queued);
        Self {
            engine,
            recovery,
            actor,
            policy,
            scope,
            job,
            _directory: directory,
        }
    }

    fn run(&self, panic_after_birth: bool) -> Result<JobRecord, EngineError> {
        let born = Arc::new(AtomicBool::new(false));
        let observed_birth = Arc::clone(&born);
        scan_worker_runtime_hooks::at_launch(move |_child| {
            observed_birth.store(true, Ordering::SeqCst);
            if panic_after_birth {
                std::panic::panic_any(String::from("macOS Engine original postbirth panic"));
            }
        });
        let observed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.engine
                .run_job(&self.job.job_id, "macos-installed-candidate-owner")
        }));
        // 先处理真实原owner，之后才能断言结果或继续原panic；不靠Engine Drop释放责任。
        let drained = self.recovery.drain();
        if !matches!(drained, Ok(true)) {
            if let Some(host) = &self.engine.scan_worker {
                std::mem::forget(Arc::clone(host));
            }
            if let Err(payload) = observed {
                std::panic::resume_unwind(payload);
            }
            panic!("Engine fixture original owner retained after failed recovery");
        }
        assert_eq!(self.recovery.occupied_slots().unwrap(), 0);
        assert!(
            born.load(Ordering::SeqCst),
            "must observe real installed child birth; setup rejection is not response-budget evidence: {observed:?}"
        );
        match observed {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn assert_staging_empty(&self, fence: u64) {
        let graph = self.engine.graph.lock().unwrap();
        assert_eq!(
            graph
                .staging_node_count(&format!("{}:{fence}", self.job.job_id))
                .unwrap(),
            0
        );
    }
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing explicit fixture input: {name}"))
}

#[test]
#[ignore = "requires ordinary UID ephemeral macOS CI installed candidate fixture"]
fn macos_engine_installed_scan_publishes_revision_and_relations() {
    let fixture = MacosEngineScanFixture::new(8 << 20);
    let result = fixture.run(false);
    let completed = result.unwrap();
    assert_eq!(completed.state, JobState::Completed);
    assert!(completed.fencing_token > 0);
    let revision = fixture
        .engine
        .revision_for_job(&fixture.job.job_id, &fixture.actor, &fixture.policy)
        .unwrap();
    assert_eq!(
        fixture.engine.latest_revision(&fixture.scope).unwrap(),
        Some(revision.clone())
    );
    let root = fixture.engine.revision_root_node(&revision).unwrap();
    let first = fixture
        .engine
        .revision_node_at(&revision, Path::new("one"))
        .unwrap()
        .unwrap();
    let nested = fixture
        .engine
        .revision_node_at(&revision, Path::new("nested"))
        .unwrap()
        .unwrap();
    let second = fixture
        .engine
        .revision_node_at(&revision, Path::new("nested/two"))
        .unwrap()
        .unwrap();
    assert_eq!(first.parent_id, Some(root.id));
    assert_eq!(nested.parent_id, Some(root.id));
    assert_eq!(second.parent_id, Some(nested.id));
    assert_eq!(first.direct_bytes, 3);
    assert_eq!(second.direct_bytes, 5);
    assert_eq!(root.subtree_bytes, 8);
    assert_eq!(root.files, 2);
    assert!(!first.read_error && !second.read_error);
    let (children, next, unknown) = fixture
        .engine
        .revision_children_page(&revision, root.id, None, 0, 10)
        .unwrap();
    assert_eq!(children.len(), 2);
    assert!(next.is_none());
    assert_eq!(unknown, 0);
    fixture.assert_staging_empty(completed.fencing_token);
    println!("DISKGRAPH_MACOS_ENGINE_FIXTURE published files=2 bytes=8 parents=3 recovery_slots=0");
}

#[test]
#[ignore = "requires ordinary UID ephemeral macOS CI installed candidate fixture"]
fn macos_engine_response_exhaustion_has_no_partial_revision() {
    let fixture = MacosEngineScanFixture::new(EXHAUSTED_RESPONSE_BYTES);
    let result = fixture.run(false);
    assert!(
        matches!(
            result.as_ref().err().map(EngineError::primary),
            Some(EngineError::Business(
                diskgraph_core::BusinessError::BudgetExceeded
            ))
        ),
        "actual born helper must fail the original response budget: {result:?}"
    );
    let failed = fixture.engine.job_status(&fixture.job.job_id).unwrap();
    assert_eq!(failed.state, JobState::Failed);
    assert!(
        fixture
            .engine
            .latest_revision(&fixture.scope)
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .engine
            .revision_for_job(&fixture.job.job_id, &fixture.actor, &fixture.policy)
            .is_err()
    );
    fixture.assert_staging_empty(failed.fencing_token);
}

#[test]
fn macos_response_fixture_admits_terminal_but_rejects_actual_tree_encoding() {
    use diskgraph_scan_worker::{
        ExecutionFrame, FrameWriter, ProtocolBudgetError, WorkerFailure, WorkerRequest,
        write_tree_with_limits,
    };
    let directory = tempfile::tempdir().unwrap();
    let root = fixture_root(directory.path());
    let options = diskgraph_disktree_core::scan::ScanOptions {
        apparent_size: true,
        ..diskgraph_disktree_core::scan::ScanOptions::default()
    };
    let limits = ProtocolLimits {
        max_frame_bytes: 64 << 10,
        max_stream_bytes: EXHAUSTED_RESPONSE_BYTES,
        max_nodes: 1000,
        max_depth: 32,
    };
    // 使用真实请求准入，覆盖内部Hello及所有固定预算终态的预留，不能只验证limits结构。
    WorkerRequest::scan(&root, &options, limits)
        .unwrap()
        .into_scan()
        .unwrap();
    assert!(
        WorkerRequest::scan(
            &root,
            &options,
            ProtocolLimits {
                max_stream_bytes: 32,
                ..limits
            }
        )
        .is_err(),
        "original 32-byte fixture fails before birth"
    );
    let tree = diskgraph_disktree_core::scan::scan(&root, options).unwrap();
    assert_eq!((tree.files, tree.dirs, tree.bytes), (2, 2, 8));
    let mut complete = Vec::new();
    assert_eq!(
        write_tree_with_limits(
            &mut complete,
            &tree,
            ProtocolLimits {
                max_stream_bytes: 8 << 20,
                ..limits
            }
        )
        .unwrap(),
        4
    );
    // 仅真实树帧已超过预算；加入Hello/Progress只会增加实际helper响应。
    assert!(complete.len() as u64 > EXHAUSTED_RESPONSE_BYTES);
    let error = write_tree_with_limits(&mut Vec::new(), &tree, limits).unwrap_err();
    assert!(
        error
            .get_ref()
            .is_some_and(|source| source.is::<ProtocolBudgetError>())
    );
    let mut terminal = Vec::new();
    let mut writer = FrameWriter::new(&mut terminal, limits);
    writer
        .write_payload(&ExecutionFrame::<String, String>::Hello {
            version: 2,
            target: env!("DISKGRAPH_ENGINE_TARGET").to_owned(),
            pin: "158f9cc2f0b332194a3ffc5acec47760c99146d8".to_owned(),
        })
        .unwrap();
    writer
        .write_payload(&WorkerFailure::new("output", error))
        .unwrap();
    writer.flush().unwrap();
    assert!(writer.bytes_written() <= EXHAUSTED_RESPONSE_BYTES);
    println!(
        "response fixture actual tree={} bytes, admitted stream={EXHAUSTED_RESPONSE_BYTES}, hello+budget-terminal={}",
        complete.len(),
        writer.bytes_written()
    );
}

#[test]
#[ignore = "requires ordinary UID ephemeral macOS CI installed candidate fixture"]
fn macos_engine_postbirth_panic_preserves_payload_and_recovery() {
    let fixture = MacosEngineScanFixture::new(8 << 20);
    let panic =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fixture.run(true))).unwrap_err();
    assert_eq!(
        *panic.downcast::<String>().unwrap(),
        "macOS Engine original postbirth panic"
    );
    assert!(
        fixture
            .engine
            .latest_revision(&fixture.scope)
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.recovery.occupied_slots().unwrap(), 0);
}
