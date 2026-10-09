//! 单次恢复必须保留同一原 owner/容量；真实 private-session child，不使用伪 child。
use super::unix_normal_exit_test_support as fixture;
use crate::ScanWorkerRecovery;
use crate::scan_worker_registry::ScanWorkerRegistry;
use std::time::{Duration, Instant};

/// 连续轮询真实退出过渡，不能把短暂未退场误报为完成或丢失原 owner。
#[test]
fn rapid_recovery_polling_keeps_original_owner_until_actual_reap() {
    for round in 0..128 {
        let directory = tempfile::tempdir().unwrap();
        let child = fixture::spawn("live_end", directory.path());
        let pid = fixture::qualified_identity(directory.path());
        let registry = ScanWorkerRegistry::new(1).unwrap();
        let reservation = registry.reserve().unwrap();
        reservation.retain(child);
        drop(reservation);
        let recovery = ScanWorkerRecovery::new(std::sync::Arc::clone(&registry));
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut failure = None;
        loop {
            match recovery.drain_until(deadline) {
                Ok(true) => break,
                Ok(false) => {
                    assert_eq!(recovery.occupied_slots().unwrap(), 1);
                    assert!(registry.reserve().is_err());
                    assert!(Instant::now() < deadline, "round {round} did not retire");
                }
                Err(error) => {
                    // 先核实并回收仍保留的原 owner，再报告错误，避免失败测试遗留进程。
                    assert_eq!(recovery.occupied_slots().unwrap(), 1);
                    assert!(registry.reserve().is_err());
                    failure = Some(error);
                    assert!(recovery.drain().unwrap());
                    break;
                }
            }
        }
        fixture::reaped(pid);
        assert_eq!(recovery.occupied_slots().unwrap(), 0);
        assert!(failure.is_none(), "round {round}: {failure:?}");
    }
}

#[test]
fn expired_recovery_deadline_retains_live_original_and_capacity_without_signaling() {
    let directory = tempfile::tempdir().unwrap();
    let child = fixture::spawn("live_end", directory.path());
    let pid = fixture::qualified_identity(directory.path());
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let reservation = registry.reserve().unwrap();
    reservation.retain(child);
    drop(reservation);
    let recovery = ScanWorkerRecovery::new(std::sync::Arc::clone(&registry));
    for _ in 0..3 {
        assert!(
            !recovery
                .drain_until(Instant::now() - Duration::from_secs(1))
                .unwrap()
        );
        assert_eq!(recovery.occupied_slots().unwrap(), 1);
        assert!(registry.reserve().is_err());
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as u32,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            },
            0
        );
        assert_eq!(
            unsafe { info.si_pid() },
            0,
            "expired drain terminated original child"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !recovery.drain_until(deadline).unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    fixture::reaped(pid);
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(registry.reserve().is_ok());
}
