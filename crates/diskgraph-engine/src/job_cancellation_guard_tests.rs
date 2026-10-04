//! 原代次退出不能删除后来代次的取消标志；来源：Engine 同一状态 owner 的真实 Arc 生命周期。
use crate::job_cancellation_guard::JobCancellationGuard;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[test]
fn old_cancellation_guard_preserves_the_replacement_generation() {
    let old = Arc::new(AtomicBool::new(false));
    let current = Arc::new(AtomicBool::new(false));
    let entries = Mutex::new(HashMap::from([("job".into(), Arc::clone(&old))]));
    let guard = JobCancellationGuard {
        entries: &entries,
        job_id: "job".into(),
        flag: old,
    };
    entries
        .lock()
        .unwrap()
        .insert("job".into(), Arc::clone(&current));
    drop(guard);
    assert!(Arc::ptr_eq(
        entries.lock().unwrap().get("job").unwrap(),
        &current
    ));
    drop(JobCancellationGuard {
        entries: &entries,
        job_id: "job".into(),
        flag: current,
    });
    assert!(entries.lock().unwrap().is_empty());
}
