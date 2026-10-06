//! 真实认领正控与历史过期状态的优先级回归；来源：Control 授权和实际 Engine fence。
//! 历史目标由独占库显式插入，Session 准入时刻只在测试构造期间建模，不声称原自然到期链路不变。
//! 合法协议 epoch 仅供控制事务使用，测试绝不打开资源、模拟 holder 或声称本机原生资格。
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use crate::Engine;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
mod historical_running_seed;

use crate::native_process::ProcessNativeSession;
use crate::{EngineConfig, EngineError};
use diskgraph_core::{
    BusinessError, IndexedFileEpoch, JobRequestAuthority, Permission, PrincipalId,
    ProcessEvidenceFailureCode as Code, ProcessEvidenceJobInput, ProcessEvidenceLimits,
    ProcessObservationMethod,
};
use diskgraph_store::{JobRecord, JobState, StoreError};
use std::cell::Cell;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use historical_running_seed::HistoricalRunningSeed;

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
fn live_public_claim_and_original_authority_reach_fence_work() {
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
    let reached = Cell::new(0);
    // 独立正控真实入队和 strict claim 后穿过同一 helper，不能拒绝所有请求冒充正确。
    f.fence(deadline, &session, "original-owner", |lease| {
        assert!(lease > now() * 1000 + 1000);
        reached.set(reached.get() + 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(reached.get(), 1);
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .job_request_authority(&f.job.job_id)
            .unwrap(),
        Some(f.authority.clone())
    );
}

#[test]
fn original_expiry_remains_authority_denied_when_execution_deadline_already_elapsed() {
    // 原真实认领覆盖由独立正控保留。此目标是已认领后认证到期的历史状态，不让五秒认证与提交抢跑。
    let f = FenceFixture::new(60);
    let historical = HistoricalRunningSeed::new(
        &f.engine,
        &f._data.path().join("diskgraph-control.sqlite"),
        &f.job,
        &f.authority,
    );
    let expiry = historical.authority.expires_at_unix_seconds().unwrap();
    assert_eq!(
        historical.authority.validate_at(now()),
        Err(BusinessError::PermissionDenied)
    );
    let started = Instant::now();
    let deadline = started + Duration::from_millis(f.limits.max_duration_ms());
    let cancel = AtomicBool::new(false);
    // 只在 Session::new 建模历史准入时刻；成功后永久恢复真实时钟，目标拒绝来自真实已过的原 exp。
    let admission_time = Cell::new(Some(expiry.checked_sub(1).unwrap()));
    let authority_check = || {
        historical
            .authority
            .validate_at(admission_time.get().unwrap_or_else(now))
            .map_err(|_| Code::PermissionDenied)
    };
    let session = ProcessNativeSession::new(&f.limits, started, &cancel, &authority_check).unwrap();
    admission_time.set(None);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(admission_time.get().is_none());
    assert!(Instant::now() >= deadline);
    assert_eq!(
        historical.authority.validate_at(now()),
        Err(BusinessError::PermissionDenied)
    );
    assert!(
        historical.job.lease_expires_unix_ms > now() * 1000 + 1000,
        "historical seed lease expired before the required fence boundary"
    );
    // 先证明原会话的 Timeout 已锁存，再要求实际 Engine fence 保留已知原认证拒绝的更高优先级。
    assert_eq!(session.check(), Err(Code::Timeout));
    let reached = Cell::new(false);
    let result = crate::process_execution_fence::checked(
        &f.engine,
        &historical.job.job_id,
        &historical.job.owner,
        historical.job.fencing_token,
        deadline,
        &session,
        |_| {
            reached.set(true);
            Ok(())
        },
    );
    assert!(!reached.get(), "expired work must never execute");
    assert!(
        matches!(&result, Err(EngineError::Store(StoreError::Conflict(message))) if message == "job request authority denied"),
        "actual stage result: {result:?}"
    );
    let control = f.engine.control_store().unwrap();
    assert_eq!(
        control
            .job_request_authority(&historical.job.job_id)
            .unwrap(),
        Some(historical.authority.clone())
    );
    assert_eq!(control.job(&historical.job.job_id).unwrap(), historical.job);
    assert_eq!(
        control.job_request_authority(&f.job.job_id).unwrap(),
        Some(f.authority.clone())
    );
}

#[test]
fn wrong_owner_is_rejected_before_unexpired_fence_work() {
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
    assert!(!reached.get());
    let current = f.engine.job_status(&f.job.job_id).unwrap();
    assert_eq!(current.state, JobState::Running);
    assert_eq!(current.owner, f.job.owner);
    assert_eq!(current.fencing_token, f.job.fencing_token);
}

#[test]
fn current_owner_and_live_metadata_grant_still_gate_unexpired_work() {
    let f = FenceFixture::new(60);
    f.engine
        .control_store()
        .unwrap()
        .revoke_grant(
            f.authority.principal(),
            &Permission::MetadataRead,
            &f.job.scope_id,
        )
        .unwrap();
    // 持久撤权是本次独立请求的前置状态；原一秒会话从该请求准入开始，不包含另一请求。
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
