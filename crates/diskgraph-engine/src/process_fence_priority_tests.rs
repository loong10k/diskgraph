//! 原认证与原执行期限同时过期的阶段回归；来源：真实 Control 授权/认领和实际 Engine fence。
//! 合法协议 epoch 仅供控制事务使用，测试绝不打开资源、模拟 holder 或声称本机原生资格。
use crate::native_process::ProcessNativeSession;
use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{
    IndexedFileEpoch, JobRequestAuthority, Permission, PrincipalId,
    ProcessEvidenceFailureCode as Code, ProcessEvidenceJobInput, ProcessEvidenceLimits,
    ProcessObservationMethod,
};
use diskgraph_store::{JobRecord, JobState, StoreError};
use std::cell::Cell;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 各例独占真实控制库及 scope 的纯控制阶段夹具；来源：Rust D42，不使用原生伪成功后端。
struct FenceFixture {
    engine: Engine,
    authority: JobRequestAuthority,
    job: JobRecord,
    limits: ProcessEvidenceLimits,
    _source: tempfile::TempDir,
    _data: tempfile::TempDir,
}
impl FenceFixture {
    fn new(expiry_after_seconds: u64) -> Self {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: data.path().into(),
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("process-fence-priority").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(
                source.path(),
                &principal,
                &engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        let limits =
            ProcessEvidenceLimits::new(1_000, 4 << 20, 32768, 65536, 8 << 20, 2, 64).unwrap();
        let input = ProcessEvidenceJobInput::new(
            engine.server_id().unwrap(),
            scope,
            "protocol-base-no-native-read".into(),
            1,
            ProcessObservationMethod::LinuxProcfsV1,
            IndexedFileEpoch::LinuxHandle {
                device: 1,
                inode: 1,
                filesystem_domain_sha256: [1; 32],
                handle_type: 1,
                handle_bytes: vec![1; 12],
            },
            limits.clone(),
        )
        .unwrap();
        let authority = JobRequestAuthority::authenticated_remote(
            principal,
            "fixture",
            "http",
            vec![Permission::MetadataRead, Permission::IndexWrite],
            now() + expiry_after_seconds,
        )
        .unwrap();
        let mut control = engine.control_store().unwrap();
        let queued = control
            .create_process_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap();
        let job = control
            .claim_job_once_strict(&queued.job_id, "original-owner")
            .unwrap();
        assert_eq!(job.state, JobState::Running);
        drop(control);
        Self {
            engine,
            authority,
            job,
            limits,
            _source: source,
            _data: data,
        }
    }
    fn fence(
        &self,
        deadline: Instant,
        session: &ProcessNativeSession<'_>,
        owner: &str,
        work: impl FnOnce(u64) -> diskgraph_store::Result<()>,
    ) -> Result<(), EngineError> {
        crate::process_execution_fence::checked(
            &self.engine,
            &self.job.job_id,
            owner,
            self.job.fencing_token,
            deadline,
            session,
            work,
        )
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn original_expiry_remains_authority_denied_when_execution_deadline_already_elapsed() {
    let f = FenceFixture::new(5);
    let started = Instant::now();
    let deadline = started + Duration::from_millis(f.limits.max_duration_ms());
    let cancel = AtomicBool::new(false);
    let authority_check = || {
        f.authority
            .validate_at(now())
            .map_err(|_| Code::PermissionDenied)
    };
    let session = ProcessNativeSession::new(&f.limits, started, &cancel, &authority_check).unwrap();
    let reached = Cell::new(0);
    // 实际合法原权限/原owner正控先穿过相同 helper，不能拒绝所有请求冒充正确。
    f.fence(deadline, &session, "original-owner", |lease| {
        assert!(lease > now() * 1000 + 1000);
        reached.set(reached.get() + 1);
        Ok(())
    })
    .unwrap();
    let expiry = f.authority.expires_at_unix_seconds().unwrap();
    while now() < expiry {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(Instant::now() >= deadline);
    assert!(f.authority.validate_at(now()).is_err());
    let result = f.fence(deadline, &session, "original-owner", |_| {
        reached.set(reached.get() + 1);
        Ok(())
    });
    assert_eq!(reached.get(), 1, "expired work must never execute");
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .job_request_authority(&f.job.job_id)
            .unwrap(),
        Some(f.authority.clone())
    );
    assert!(
        matches!(&result, Err(EngineError::Store(StoreError::Conflict(message))) if message == "job request authority denied"),
        "actual stage result: {result:?}"
    );
}

#[test]
fn current_owner_and_live_metadata_grant_still_gate_unexpired_work() {
    let f = FenceFixture::new(60);
    let started = Instant::now();
    let deadline = started + Duration::from_millis(f.limits.max_duration_ms());
    let cancel = AtomicBool::new(false);
    let authority_check = || {
        f.authority
            .validate_at(now())
            .map_err(|_| Code::PermissionDenied)
    };
    let session = ProcessNativeSession::new(&f.limits, started, &cancel, &authority_check).unwrap();
    let reached = Cell::new(false);
    let wrong_owner = f.fence(deadline, &session, "wrong-owner", |_| {
        reached.set(true);
        Ok(())
    });
    assert!(
        matches!(wrong_owner, Err(EngineError::Store(StoreError::StaleOwner))),
        "{wrong_owner:?}"
    );
    f.engine
        .control_store()
        .unwrap()
        .revoke_grant(
            f.authority.principal(),
            &Permission::MetadataRead,
            &f.job.scope_id,
        )
        .unwrap();
    let revoked = f.fence(deadline, &session, "original-owner", |_| {
        reached.set(true);
        Ok(())
    });
    assert!(
        matches!(&revoked, Err(EngineError::Store(StoreError::Conflict(message))) if message == "live job authorization withdrawn"),
        "{revoked:?}"
    );
    assert!(!reached.get());
    let current = f.engine.job_status(&f.job.job_id).unwrap();
    assert_eq!(current.state, JobState::Running);
    assert_eq!(current.owner, f.job.owner);
    assert_eq!(current.fencing_token, f.job.fencing_token);
}
