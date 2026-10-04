//! 产品采样分类来自实际类型化预算，不从诊断文本推断；来源：原生 Rust ProbeBudget。
use super::evidence_probe_session::EvidenceProbeSession;
use super::git_product_error::GitProductError;
use diskgraph_core::GitEvidenceLimits;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

#[test]
fn original_claim_deadline_and_cancellation_cannot_restart_in_product_session() {
    let limits = GitEvidenceLimits::default();
    let old = Instant::now() - Duration::from_millis(limits.max_duration_ms() + 1);
    assert!(matches!(
        EvidenceProbeSession::for_git_job(&limits, old, Arc::new(AtomicBool::new(false))),
        Err(GitProductError::Deadline)
    ));
    assert!(matches!(
        EvidenceProbeSession::for_git_job(&limits, Instant::now(), Arc::new(AtomicBool::new(true))),
        Err(GitProductError::Cancelled)
    ));
}
