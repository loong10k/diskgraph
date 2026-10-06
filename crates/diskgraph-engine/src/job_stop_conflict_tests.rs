//! 独立 native 身份冲突与请求拒权同时存在时，不能把真实原错误归一。
//! 来源：实际已索引 Git 目录、原生替换、成功认领代次与 PF-06 请求桥。

use crate::EngineError;
use crate::git_evidence_fixture::{GitEvidenceFixture, now};
use crate::git_evidence_target::GitEvidenceTarget;
use crate::job_request_cancel_bridge::JobRequestCancelBridge;
#[cfg(not(windows))]
use crate::live_evidence::EvidenceProbeSession;
#[cfg(windows)]
use crate::live_evidence::NativeEvidenceTestSession as EvidenceProbeSession;
use crate::live_evidence::ProbeLimits;
use diskgraph_core::BusinessError;
use diskgraph_store::StoreError;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[test]
fn observed_denial_does_not_replace_a_real_indexed_directory_identity_conflict() {
    let f = GitEvidenceFixture::new();
    let other = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let input = f
        .engine
        .control_store()
        .unwrap()
        .git_evidence_job_input(&job.job_id)
        .unwrap();
    let target = GitEvidenceTarget::load(
        &f.engine,
        &input,
        Instant::now() + Duration::from_secs(15),
        None,
    )
    .unwrap();
    // register_scope 持久化的是实际规范化根；不能把 tempfile 的 /var 等链接
    // 别名交给要求逐组件 no-follow 的 sampler，造成身份测试的前置 Unsupported。
    let root = f
        .engine
        .control_store()
        .unwrap()
        .scope(&f.scope)
        .unwrap()
        .root
        .to_native_path()
        .unwrap();
    let limits = ProbeLimits::default();
    let mut positive = EvidenceProbeSession::new(&limits).unwrap();
    assert_eq!(
        positive
            .sample_git_indexed(Path::new("git"), &root, &target.locator, &target.identity)
            .unwrap()
            .dirty_count,
        0,
        "unchanged real indexed repository must be sampleable"
    );
    // 两个仓库均由真实公开 scan 建立；保留原目录，原路径改为另一实际 Git 目录。
    // session 的原 cancel 保持 false，下面的 Conflict 必须来自 indexed identity。
    std::fs::rename(&root, f.temp.path().join("retained-indexed-repo")).unwrap();
    std::fs::rename(other.temp.path().join("repo"), &root).unwrap();
    let mut changed = EvidenceProbeSession::new(&limits).unwrap();
    let native_error = changed
        .sample_git_indexed(Path::new("git"), &root, &target.locator, &target.identity)
        .unwrap_err();
    assert!(!limits.cancel.load(Ordering::SeqCst));
    assert_eq!(native_error.business(), BusinessError::Conflict);
    assert_eq!(format!("{native_error:?}"), "IdentityChanged");

    let claimed = f
        .engine
        .control_store()
        .unwrap()
        .claim_job_once(&job.job_id, "independent-denial")
        .unwrap();
    let request = AtomicBool::new(false);
    let denied = AtomicBool::new(false);
    let local_stop = AtomicBool::new(false);
    let bridge =
        JobRequestCancelBridge::new(&f.engine, &claimed, Some((&request, &denied)), &local_stop);
    assert!(matches!(
        bridge.preserve_outcome(Err(native_error.business().into())),
        Err(EngineError::Business(BusinessError::Conflict))
    ));
    denied.store(true, Ordering::SeqCst);
    assert!(matches!(
        bridge.check(),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert!(bridge.denial_observed());
    assert!(local_stop.load(Ordering::SeqCst));
    assert!(
        !f.engine
            .control_store()
            .unwrap()
            .cancellation_requested(&job.job_id, claimed.fencing_token)
            .unwrap()
    );
    // 已观察拒权只能描述独立停止来源，不能证明 native 身份冲突由停止产生。
    let result = bridge.preserve_outcome(Err(native_error.business().into()));
    assert!(
        matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
        "independent native identity conflict was replaced by observed denial: {result:?}"
    );
    f.assert_no_git_publication();
}

#[test]
fn a_specific_local_commit_stop_can_be_denied_without_rewriting_other_store_errors() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let claimed = f
        .engine
        .control_store()
        .unwrap()
        .claim_job_once(&job.job_id, "pure-stop")
        .unwrap();
    let request = AtomicBool::new(false);
    let denied = AtomicBool::new(true);
    let local_stop = AtomicBool::new(false);
    let bridge =
        JobRequestCancelBridge::new(&f.engine, &claimed, Some((&request, &denied)), &local_stop);
    assert!(matches!(
        bridge.check(),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert!(matches!(
        bridge.preserve_outcome(Err(StoreError::Conflict(
            "scan cancelled before commit".into()
        )
        .into())),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    let other = bridge.preserve_outcome(Err(StoreError::Conflict(
        "live job authorization withdrawn".into(),
    )
    .into()));
    assert!(
        matches!(other, Err(EngineError::Store(StoreError::Conflict(message))) if message == "live job authorization withdrawn")
    );
    assert!(matches!(
        bridge.preserve_outcome(Err(StoreError::StaleOwner.into())),
        Err(EngineError::Store(StoreError::StaleOwner))
    ));
    assert!(
        bridge.preserve_outcome(Ok(())).is_ok(),
        "completed fact cannot be rewritten"
    );
}
