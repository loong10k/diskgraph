//! 原扫描互斥锁真实 poison 后，监督退休仍必须关闭另一原探针池。
use crate::{EngineError, ProbeHost, SupervisorOwner, SupervisorRecoveryError};
use diskgraph_core::BusinessError;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn original_scan_seal_failure_still_closes_probe_admission_and_retains_engine() {
    let (_directory, mut parts, registry) = crate::supervisor_binding_tests::fixture();
    let (probe, recovery) = ProbeHost::new(1).unwrap();
    let probe_registry = Arc::clone(&probe.registry);
    Arc::get_mut(&mut parts.engine).unwrap().probe_host = Some(probe);
    parts.probe = Some(recovery);
    let original_engine = Arc::clone(&parts.engine);
    let until = Instant::now() + Duration::from_secs(5);
    let mut owner = SupervisorOwner::bind(parts, until)
        .unwrap_or_else(|_| panic!("original supervisor binding failed"));
    // 捕获持有实际原 slots 锁时的 panic；不模拟返回值、不建立替代资源表。
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _original_lock = registry.slots.lock().unwrap();
            panic!("actual original scan state lock failure");
        }))
        .is_err()
    );
    assert!(matches!(
        owner.poll_retirement(until),
        Err(SupervisorRecoveryError::Engine(EngineError::Poisoned))
    ));
    assert!(Arc::ptr_eq(owner.engine().unwrap(), &original_engine));
    assert!(matches!(
        probe_registry.reserve(),
        Err(EngineError::Business(BusinessError::Conflict))
    ));
    // 再次恢复仍返回原 poison，不凭错误或空池清除 ACTIVE。
    assert!(matches!(
        owner.poll_retirement(until),
        Err(SupervisorRecoveryError::Engine(EngineError::Poisoned))
    ));
}
