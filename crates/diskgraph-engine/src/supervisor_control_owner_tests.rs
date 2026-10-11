//! 原控制通知与真实未出生容量绑定；不代替实际 child、出生认证或前端 EOF 验收。
use crate::recovery_control::{
    ControlError, ControlFrame, ControlNotification, ControlReceiver, UnixControlWriter,
};
use crate::{SupervisorControlOwner, SupervisorControlProgress, SupervisorOwner};
use std::io::{ErrorKind, Read};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn control(owner: SupervisorOwner, until: Instant) -> (SupervisorControlOwner, UnixStream) {
    let (sender, receiver) = UnixStream::pair().unwrap();
    receiver.set_nonblocking(true).unwrap();
    (
        SupervisorControlOwner::new(
            owner,
            UnixControlWriter::new(sender, [7; 32], until).unwrap(),
        ),
        receiver,
    )
}
fn read_frames(receiver: &mut UnixStream) -> Vec<ControlFrame> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match receiver.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) => panic!("private control read failed: {error}"),
        }
        assert!(bytes.len() <= 65536);
    }
    ControlReceiver::new([7; 32], deadline())
        .push(&bytes)
        .unwrap()
}
fn owner(parts: crate::SupervisorParts) -> SupervisorOwner {
    SupervisorOwner::bind(parts, deadline()).unwrap_or_else(|_| panic!("original binding failed"))
}
fn started(control: &mut SupervisorControlOwner, cancel: &AtomicBool) {
    control.ready(cancel).unwrap();
    control.foreground_ended(0, false, cancel).unwrap();
}

#[test]
fn actual_original_pending_reservation_cannot_emit_cleanup_complete() {
    let (_dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let original = registry.reserve().unwrap();
    let (mut control, mut receiver) = control(owner(parts), deadline());
    let cancel = AtomicBool::new(false);
    started(&mut control, &cancel);
    // 一百轮真实 Pending 不消耗一百帧，也不能凭 Engine 已销毁声明原池完成。
    for _ in 0..100 {
        assert_eq!(
            control.poll_retirement(deadline(), &cancel).unwrap(),
            SupervisorControlProgress::Pending {
                control_failure: None
            }
        );
    }
    let frames = read_frames(&mut receiver);
    assert_eq!(frames.len(), 3);
    assert!(matches!(
        frames[2].notification,
        ControlNotification::CleanupPending {}
    ));
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame.notification, ControlNotification::CleanupComplete {})),
        "original reservation remains pending but control claimed cleanup"
    );
    drop(original);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
    let mut bytes = [0; 4096];
    let count = receiver.read(&mut bytes).unwrap();
    // 后续只有原序列3的唯一完成帧；重复成功不重发。
    let body: ControlFrame = serde_json::from_slice(&bytes[4..count]).unwrap();
    assert_eq!(body.sequence, 3);
    assert!(matches!(
        body.notification,
        ControlNotification::CleanupComplete {}
    ));
    assert_eq!(
        receiver.read(&mut bytes).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
}

#[test]
fn external_original_engine_reference_blocks_complete_notification() {
    let (_dir, parts, _registry) = crate::supervisor_binding_tests::fixture();
    let (mut control, mut receiver) = control(owner(parts), deadline());
    let original_engine = Arc::clone(control.engine().unwrap());
    let cancel = AtomicBool::new(false);
    started(&mut control, &cancel);
    assert!(control.engine().is_none());
    assert!(matches!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Pending { .. }
    ));
    let before = read_frames(&mut receiver);
    assert_eq!(before.len(), 3);
    assert!(
        !before
            .iter()
            .any(|frame| matches!(frame.notification, ControlNotification::CleanupComplete {}))
    );
    drop(original_engine);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
}

#[test]
fn disconnected_control_retains_pending_then_reports_retired_unconfirmed() {
    let (dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let original = registry.reserve().unwrap();
    let (mut control, receiver) = control(owner(parts), deadline());
    let cancel = AtomicBool::new(false);
    drop(receiver);
    assert_eq!(control.ready(&cancel), Err(ControlError::Unconfirmed));
    assert_eq!(
        control.foreground_ended(10, true, &cancel),
        Err(ControlError::Unconfirmed)
    );
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Pending {
            control_failure: Some(ControlError::Unconfirmed)
        }
    );
    assert_eq!(
        std::fs::read(dir.path().join("slot")).unwrap(),
        b"DGSL01A\n"
    );
    drop(original);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::RetiredUnconfirmed {
            control_failure: ControlError::Unconfirmed
        }
    );
    assert_eq!(
        std::fs::read(dir.path().join("slot")).unwrap(),
        b"DGSL01C\n"
    );
}

#[test]
fn expired_control_deadline_does_not_refresh_or_prevent_original_cleanup() {
    let (dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let original = registry.reserve().unwrap();
    let (mut control, mut receiver) =
        control(owner(parts), Instant::now() - Duration::from_secs(1));
    let cancel = AtomicBool::new(false);
    assert_eq!(control.ready(&cancel), Err(ControlError::Deadline));
    assert_eq!(
        control.foreground_ended(6, false, &cancel),
        Err(ControlError::Deadline)
    );
    assert!(matches!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Pending {
            control_failure: Some(ControlError::Deadline)
        }
    ));
    drop(original);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::RetiredUnconfirmed {
            control_failure: ControlError::Deadline
        }
    );
    assert_eq!(
        std::fs::read(dir.path().join("slot")).unwrap(),
        b"DGSL01C\n"
    );
    assert!(read_frames(&mut receiver).is_empty());
}

#[test]
fn damaged_original_slot_returns_error_without_complete_notification() {
    let (dir, parts, _registry) = crate::supervisor_binding_tests::fixture();
    let (mut control, mut receiver) = control(owner(parts), deadline());
    let cancel = AtomicBool::new(false);
    started(&mut control, &cancel);
    std::fs::write(dir.path().join("slot"), b"damaged!").unwrap();
    assert!(control.poll_retirement(deadline(), &cancel).is_err());
    let frames = read_frames(&mut receiver);
    assert_eq!(frames.len(), 2);
    assert_eq!(std::fs::read(dir.path().join("slot")).unwrap(), b"damaged!");
}

#[test]
fn control_cancel_does_not_cancel_original_retirement_or_claim_notification() {
    let (dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let original = registry.reserve().unwrap();
    let (mut control, mut receiver) = control(owner(parts), deadline());
    let cancel = AtomicBool::new(false);
    started(&mut control, &cancel);
    cancel.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Pending {
            control_failure: Some(ControlError::Cancelled)
        }
    );
    drop(original);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::RetiredUnconfirmed {
            control_failure: ControlError::Cancelled
        }
    );
    assert_eq!(
        std::fs::read(dir.path().join("slot")).unwrap(),
        b"DGSL01C\n"
    );
    assert_eq!(read_frames(&mut receiver).len(), 2);
}

#[test]
fn retirement_before_foreground_end_does_not_seal_original_resources() {
    let (_dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let (mut control, mut receiver) = control(owner(parts), deadline());
    let cancel = AtomicBool::new(false);
    control.ready(&cancel).unwrap();
    assert!(control.poll_retirement(deadline(), &cancel).is_err());
    assert!(registry.reserve().is_ok());
    assert_eq!(read_frames(&mut receiver).len(), 1);
    control.foreground_ended(0, false, &cancel).unwrap();
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
}

#[test]
fn repeated_ready_latches_protocol_failure_without_releasing_original_owner() {
    let (_dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let (mut control, mut receiver) = control(owner(parts), deadline());
    let cancel = AtomicBool::new(false);
    control.ready(&cancel).unwrap();
    assert_eq!(control.ready(&cancel), Err(ControlError::Protocol));
    assert!(registry.reserve().is_ok());
    assert_eq!(
        control.foreground_ended(0, false, &cancel),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::RetiredUnconfirmed {
            control_failure: ControlError::Protocol
        }
    );
    assert_eq!(read_frames(&mut receiver).len(), 1);
}

#[test]
fn lifecycle_expired_round_defers_unstarted_notification_without_poisoning_channel() {
    expired_lifecycle_round(true);
}

#[test]
fn lifecycle_retired_resources_defer_unstarted_completion_on_expired_round() {
    expired_lifecycle_round(false);
}

fn expired_lifecycle_round(pending: bool) {
    let (_dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let original = pending.then(|| registry.reserve().unwrap());
    let (sender, mut receiver) = UnixStream::pair().unwrap();
    receiver.set_nonblocking(true).unwrap();
    let mut control = SupervisorControlOwner::new(
        owner(parts),
        UnixControlWriter::new_lifecycle(sender, [7; 32]).unwrap(),
    );
    let cancel = AtomicBool::new(false);
    control.ready_lifecycle(&cancel, deadline()).unwrap();
    control
        .foreground_ended_lifecycle(0, false, &cancel, deadline())
        .unwrap();
    let until = Instant::now() + Duration::from_millis(20);
    // 原池真实轮询之后模拟调度停顿，确保耗尽的是通知阶段而非入口检查。
    control.after_retirement_poll = Some(Box::new(move || {
        std::thread::sleep(until.saturating_duration_since(Instant::now()));
    }));
    assert_eq!(
        control.poll_retirement(until, &cancel).unwrap(),
        SupervisorControlProgress::Pending {
            control_failure: None
        }
    );
    drop(original);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match receiver.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) => panic!("private control read failed: {error}"),
        }
    }
    let frames = ControlReceiver::new_lifecycle([7; 32])
        .push(&bytes)
        .unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[2].sequence, 2);
    assert!(matches!(
        frames[2].notification,
        ControlNotification::CleanupComplete {}
    ));
}

#[test]
fn executor_control_completion_preserves_guardian_active_slot_and_original_pending() {
    let (directory, parts, registry) = crate::supervisor_binding_tests::fixture();
    let crate::SupervisorParts {
        engine,
        scan,
        slot: mut guardian_slot,
        #[cfg(windows)]
        probe,
    } = parts;
    let pending = registry.reserve().unwrap();
    let resource_owner = crate::ExecutorResourceOwner::bind(crate::ExecutorParts {
        engine,
        scan,
        #[cfg(windows)]
        probe,
    })
    .unwrap_or_else(|_| panic!("original executor binding rejected"));
    let (sender, mut receiver) = UnixStream::pair().unwrap();
    receiver.set_nonblocking(true).unwrap();
    let mut control = SupervisorControlOwner::for_executor(
        resource_owner,
        UnixControlWriter::new_lifecycle(sender, [7; 32]).unwrap(),
    );
    let cancel = AtomicBool::new(false);
    control.ready_lifecycle(&cancel, deadline()).unwrap();
    control
        .foreground_ended_lifecycle(0, false, &cancel, deadline())
        .unwrap();
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Pending {
            control_failure: None
        }
    );
    guardian_slot.verify_active(deadline()).unwrap();
    assert_eq!(registry.occupied().unwrap(), 1);
    drop(pending);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
    guardian_slot.verify_active(deadline()).unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("slot")).unwrap(),
        b"DGSL01A\n"
    );
    let foreign = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("slot"))
        .unwrap();
    assert!(matches!(
        crate::recovery_slot::SlotReservation::acquire(foreign, deadline()),
        Err(crate::recovery_slot::SlotError::Busy)
    ));
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match receiver.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) => panic!("original private channel read failed: {error}"),
        }
    }
    let frames = ControlReceiver::new_lifecycle([7; 32])
        .push(&bytes)
        .unwrap();
    assert_eq!(frames.len(), 4);
    assert!(matches!(
        frames[2].notification,
        ControlNotification::CleanupPending {}
    ));
    assert!(matches!(
        frames[3].notification,
        ControlNotification::CleanupComplete {}
    ));
}
