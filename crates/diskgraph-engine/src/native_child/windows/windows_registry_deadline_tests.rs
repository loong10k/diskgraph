//! 原生 Windows Registry 期限与原 owner 责任；缺 API 编译失败仅是开发 RED。
use super::windows_child::WindowsChild;
use super::windows_cleanup_hooks::WindowsCleanupHooks as Hooks;
use super::windows_cleanup_rescue::WindowsCleanupRescue;
use super::windows_control_fixture::command;
use crate::native_child::ChildInputMode;
use crate::scan_worker_registry::ScanWorkerRegistry;
use crate::{EngineError, ScanWorkerRecovery};
use diskgraph_core::BusinessError;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn retained_case(stage: Option<u8>) {
    let directory = tempfile::tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut owner = None;
    let birth = WindowsChild::spawn_into(
        &mut command("hold", directory.path()),
        ChildInputMode::WorkerControl,
        &mut owner,
        || {
            if Instant::now() < deadline {
                Ok(())
            } else {
                Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
            }
        },
    );
    if let Err(error) = birth {
        if let Some(child) = owner.as_mut() {
            let _ = child.cleanup();
        }
        panic!("real child birth failed: {error:?}");
    }
    let mut child = owner.take().unwrap();
    let rescue = match WindowsCleanupRescue::capture(&child) {
        Ok(rescue) => rescue,
        Err(error) => {
            let cleanup = child.cleanup();
            panic!("guardian capture failed: {error:?}; cleanup={cleanup:?}");
        }
    };
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let recovery = ScanWorkerRecovery::new(Arc::clone(&registry));
    let reservation = registry.reserve().unwrap();
    reservation.retain(child);
    drop(reservation);
    let observed = catch_unwind(AssertUnwindSafe(|| {
        assert!(!rescue.waited().unwrap());
        assert!(rescue.active().unwrap() > 0);
        Hooks::arm(stage.unwrap_or(1));
        if stage.is_some() {
            let error = recovery.drain_until(deadline).unwrap_err();
            assert!(matches!(error, EngineError::Io(ref error) if error.raw_os_error() == Some(5)));
            assert_eq!(
                Hooks::counts().2,
                1,
                "intended one-shot boundary fault must be consumed"
            );
        } else {
            assert!(!recovery.drain_until(Instant::now()).unwrap());
            assert_eq!(
                Hooks::counts(),
                (0, 0, 0),
                "expired admission must not touch original native owner"
            );
            assert!(!rescue.waited().unwrap());
            assert!(rescue.active().unwrap() > 0);
            Hooks::disarm();
        }
        assert_eq!(recovery.occupied_slots().unwrap(), 1);
        assert!(matches!(
            registry.reserve(),
            Err(EngineError::Business(BusinessError::ResourceExhausted))
        ));
        let before = Hooks::counts();
        loop {
            assert!(
                Instant::now() < deadline,
                "original cleanup deadline exhausted"
            );
            if recovery.drain_until(deadline).unwrap() {
                break;
            }
            assert_eq!(recovery.occupied_slots().unwrap(), 1);
            assert!(
                registry.reserve().is_err(),
                "Pending must not refund capacity"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let after = Hooks::counts();
        assert!(
            after.0 > before.0 && after.1 > before.1,
            "retry must use real original wait and Job accounting"
        );
        assert!(rescue.waited().unwrap());
        assert_eq!(rescue.active().unwrap(), 0);
        assert_eq!(recovery.occupied_slots().unwrap(), 0);
        drop(registry.reserve().unwrap());
    }));
    Hooks::disarm();
    // 显式安全 finally 使用旧兼容清理；不把这个可能阻塞的路径计为有限返回证明。
    let rescued = rescue.finish();
    let drained = recovery.drain();
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
    rescued.unwrap();
    assert!(drained.unwrap());
}

#[test]
fn expired_registry_drain_retains_original_owner_and_capacity_until_actual_retry() {
    retained_case(None);
}
#[test]
fn registry_deadline_wait_error_preserves_original_owner_and_capacity() {
    retained_case(Some(1));
}
#[test]
fn registry_deadline_query_error_preserves_original_owner_and_capacity() {
    retained_case(Some(2));
}
