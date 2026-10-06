use crate::native_child::{LinuxAtomicChild, LinuxAtomicLauncher};
use crate::scan_worker_registry::ScanWorkerRegistry;
use crate::{
    Engine, EngineConfig, EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRecovery,
    ScanWorkerRuntimeBudget, admin_scope,
};
use diskgraph_core::{
    Authorizer, BusinessError, Decision, Permission, PolicyAuthorizer, PrincipalId, ScopeId,
};
use diskgraph_scan_worker::ProtocolLimits;
use diskgraph_store::JobRecord;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 原helper/真实持久claim/外部recovery夹具；来源：PF-06，错误来自真实内核waitid策略。
pub(super) struct ScanWorkerRecoveryFixture {
    pub engine: Arc<Engine>,
    pub recovery: ScanWorkerRecovery,
    pub registry: Arc<ScanWorkerRegistry>,
    pub job: JobRecord,
    pub scope: ScopeId,
    image: PathBuf,
    _directory: tempfile::TempDir,
}

impl ScanWorkerRecoveryFixture {
    /// 参数：无；返回：明确Cargo artifact、公开授权/Queued任务和原容量1的真实宿主。
    pub(super) fn new() -> Self {
        let image = match std::env::var_os("DISKGRAPH_ENGINE_SCAN_WORKER") {
            Some(path) => PathBuf::from(path),
            None => panic!("root must provide the actual Cargo worker artifact explicitly"),
        };
        assert!(image.is_absolute() && image.is_file());
        let mut held = File::open(&image).unwrap();
        let length = held.metadata().unwrap().len();
        assert!(length > 0 && length <= 128 << 20);
        let mut hash = Sha256::new();
        let mut bytes = [0; 64 << 10];
        loop {
            let n = held.read(&mut bytes).unwrap();
            if n == 0 {
                break;
            }
            hash.update(&bytes[..n]);
        }
        let config =
            ScanWorkerHostConfig::from_expected_image(hash.finalize().into(), length).unwrap();
        let host = ScanWorkerHost::new(
            held,
            config,
            ScanWorkerRuntimeBudget::new(
                ProtocolLimits {
                    max_frame_bytes: 64 << 10,
                    max_stream_bytes: 8 << 20,
                    max_nodes: 1000,
                    max_depth: 32,
                },
                0,
                1,
            )
            .unwrap(),
        )
        .unwrap();
        let registry = Arc::clone(&host.registry);
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("scope");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("one"), b"real physical worker file").unwrap();
        let (engine, recovery) = Engine::open_with_scan_worker(
            EngineConfig {
                data_dir: directory.path().join("data"),
                ..EngineConfig::default()
            },
            host,
        )
        .unwrap();
        let actor = PrincipalId::new("physical-recovery-test").unwrap();
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
        policy.grant(actor.clone(), Permission::IndexWrite, scope.clone());
        assert_eq!(
            engine
                .control_store()
                .unwrap()
                .live_permission(&actor, &Permission::IndexWrite, &scope)
                .unwrap(),
            Some(true)
        );
        let job = engine.index_scope(&scope, &actor, &policy).unwrap();
        Self {
            engine: Arc::new(engine),
            recovery,
            registry,
            job,
            scope,
            image,
            _directory: directory,
        }
    }

    /// 参数：无；返回：原Atomic生产入口真实owner，helper尚在stdin等待Request。
    pub(super) fn launch_original(&self) -> LinuxAtomicChild {
        let end = Instant::now() + Duration::from_secs(20);
        let prepared = LinuxAtomicLauncher::prepare(
            File::open(&self.image).unwrap(),
            vec![CString::new("diskgraph-scan-worker").unwrap()],
            Vec::new(),
        )
        .unwrap();
        let mut unwind_owner = None;
        match prepared.spawn(
            end,
            &mut || {
                if Instant::now() >= end {
                    Err(EngineError::from(BusinessError::BudgetExceeded))
                } else {
                    Ok(())
                }
            },
            &mut unwind_owner,
        ) {
            Ok(child) => child,
            Err(failure) => {
                let (error, mut owner) = failure.into_parts();
                if let Some(child) = owner.as_mut() {
                    child.cleanup().unwrap();
                }
                panic!("actual Atomic launch qualification failed: {error:?}");
            }
        }
    }
}
