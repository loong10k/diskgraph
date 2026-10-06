//! 真实wait拒绝后的同owner/容量保持与Engine unwind外恢复；来源：PF-06。
//! seccomp仅施加到测试调用线程，已有未过滤宿主仍可消费原wait，不注入错误对象。

use crate::scan_worker_recovery_fixture::ScanWorkerRecoveryFixture;
use crate::{EngineError, scan_worker_runtime_hooks};
use diskgraph_core::BusinessError;
use diskgraph_store::JobState;
use std::sync::Arc;

pub(crate) fn deny_waitid_for_this_thread() {
    let code = [
        libc::sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0,
        },
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: libc::SYS_waitid as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ERRNO | libc::EACCES as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ALLOW,
        },
    ];
    let program = libc::sock_fprog {
        len: code.len() as u16,
        filter: code.as_ptr().cast_mut(),
    };
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
        0
    );
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) },
        0
    );
}

#[test]
fn actual_wait_denial_retains_stopped_unreaped_owner_and_original_capacity() {
    let fixture = ScanWorkerRecoveryFixture::new();
    let reserved = fixture.registry.reserve().unwrap();
    let child = fixture.launch_original();
    assert!(!child.physically_exited().unwrap());
    assert!(!child.reaped());
    reserved.retain(child);
    drop(reserved);
    let registry = Arc::clone(&fixture.registry);
    let error = std::thread::spawn(move || {
        deny_waitid_for_this_thread();
        registry.drain().unwrap_err()
    })
    .join()
    .unwrap();
    let occupied = fixture.recovery.occupied_slots();
    let admission = matches!(
        fixture.registry.reserve(),
        Err(EngineError::Business(BusinessError::ResourceExhausted))
    );
    // 未过滤宿主使用同registry中的原owner实际reap，所有断言前完成处置。
    let drained = fixture.recovery.drain();
    assert!(
        matches!(error.primary(), EngineError::Io(source) if source.raw_os_error() == Some(libc::EACCES)),
        "{error:?}"
    );
    assert_eq!(
        occupied.unwrap(),
        1,
        "physical stop must not release unreaped slot"
    );
    assert!(admission);
    assert!(drained.unwrap());
    assert_eq!(fixture.recovery.occupied_slots().unwrap(), 0);
}

#[test]
fn engine_unwind_returns_original_payload_and_child_to_external_recovery() {
    let fixture = ScanWorkerRecoveryFixture::new();
    let engine = Arc::clone(&fixture.engine);
    let id = fixture.job.job_id.clone();
    let original = Box::new(*b"ORIGINAL");
    let address = original.as_ptr() as usize;
    let joined = std::thread::spawn(move || {
        scan_worker_runtime_hooks::at_launch(move |child| {
            assert!(!child.physically_exited().unwrap());
            assert!(!child.reaped());
            deny_waitid_for_this_thread();
            std::panic::panic_any(original);
        });
        engine.run_job(&id, "real-unwind-owner")
    })
    .join();
    let occupied = fixture.recovery.occupied_slots();
    let admission = matches!(
        fixture.registry.reserve(),
        Err(EngineError::Business(BusinessError::ResourceExhausted))
    );
    let terminal = fixture.engine.job_status(&fixture.job.job_id);
    let latest = fixture.engine.latest_revision(&fixture.scope);
    // 原Recovery在catch/join之外真实保留，并由未过滤线程恢复；不换panic payload。
    let drained = fixture.recovery.drain();
    let payload = joined.expect_err("specified panic must reach host unchanged");
    let same = payload
        .downcast::<Box<[u8; 8]>>()
        .expect("original boxed payload type");
    assert_eq!(same.as_ref().as_ptr() as usize, address);
    assert_eq!(same.as_ref().as_ref(), b"ORIGINAL");
    assert_eq!(occupied.unwrap(), 1);
    assert!(admission);
    assert_eq!(terminal.unwrap().state, JobState::Running);
    assert_eq!(latest.unwrap(), None);
    assert!(drained.unwrap());
    assert_eq!(fixture.recovery.occupied_slots().unwrap(), 0);
    assert!(fixture.engine.cancellations.lock().unwrap().is_empty());
}
