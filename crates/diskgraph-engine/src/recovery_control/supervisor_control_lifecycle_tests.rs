//! 版本2私有通知与同一原Engine/Recovery/ACTIVE退休链路的实际验收。
use crate::recovery_control::{
    ControlError, ControlNotification, ControlReceiver, UnixControlWriter,
};
use crate::{SupervisorControlOwner, SupervisorControlProgress, SupervisorOwner};
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

#[test]
fn original_pending_owner_can_notify_and_complete_with_lifecycle_writer() {
    let (_dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let original = registry.reserve().unwrap();
    let owner = SupervisorOwner::bind(parts, deadline())
        .unwrap_or_else(|_| panic!("original owner binding"));
    let (sender, mut receiver) = UnixStream::pair().unwrap();
    let mut control = SupervisorControlOwner::new(
        owner,
        UnixControlWriter::new_lifecycle(sender, [7; 32]).unwrap(),
    );
    let cancel = AtomicBool::new(false);
    control.ready_lifecycle(&cancel, deadline()).unwrap();
    control
        .foreground_ended_lifecycle(0, false, &cancel, deadline())
        .unwrap();
    for _ in 0..100 {
        assert_eq!(
            control.poll_retirement(deadline(), &cancel).unwrap(),
            SupervisorControlProgress::Pending {
                control_failure: None
            }
        );
    }
    drop(original);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
    drop(control);
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    let mut parser = ControlReceiver::new_lifecycle([7; 32]);
    let notifications: Vec<_> = parser
        .push(&bytes)
        .unwrap()
        .into_iter()
        .map(|frame| frame.notification)
        .collect();
    assert_eq!(
        notifications,
        vec![
            ControlNotification::Ready {},
            ControlNotification::ForegroundEnded {
                exit_code: 0,
                panicked: false
            },
            ControlNotification::CleanupPending {},
            ControlNotification::CleanupComplete {}
        ]
    );
    parser.end_of_stream().unwrap();
    assert!(registry.reserve().is_err());
}

#[test]
fn lifecycle_completion_waits_for_the_original_external_engine_reference() {
    let (_dir, parts, _registry) = crate::supervisor_binding_tests::fixture();
    let owner =
        SupervisorOwner::bind(parts, deadline()).unwrap_or_else(|_| panic!("original owner"));
    let (sender, mut receiver) = UnixStream::pair().unwrap();
    let mut control = SupervisorControlOwner::new(
        owner,
        UnixControlWriter::new_lifecycle(sender, [7; 32]).unwrap(),
    );
    let original_engine = Arc::clone(control.engine().unwrap());
    let cancel = AtomicBool::new(false);
    control.ready_lifecycle(&cancel, deadline()).unwrap();
    control
        .foreground_ended_lifecycle(0, false, &cancel, deadline())
        .unwrap();
    assert!(control.engine().is_none());
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Pending {
            control_failure: None
        }
    );
    drop(original_engine);
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Complete
    );
    drop(control);
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    let mut parser = ControlReceiver::new_lifecycle([7; 32]);
    assert_eq!(parser.push(&bytes).unwrap().len(), 4);
    parser.end_of_stream().unwrap();
}

#[test]
fn lifecycle_notification_expiry_keeps_active_until_original_resource_retirement() {
    let (dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let original = registry.reserve().unwrap();
    let owner =
        SupervisorOwner::bind(parts, deadline()).unwrap_or_else(|_| panic!("original owner"));
    let (sender, mut receiver) = UnixStream::pair().unwrap();
    let mut control = SupervisorControlOwner::new(
        owner,
        UnixControlWriter::new_lifecycle(sender, [7; 32]).unwrap(),
    );
    let cancel = AtomicBool::new(false);
    control.ready_lifecycle(&cancel, deadline()).unwrap();
    assert_eq!(
        control.foreground_ended_lifecycle(
            0,
            false,
            &cancel,
            Instant::now() - Duration::from_secs(1)
        ),
        Err(ControlError::Deadline)
    );
    assert!(control.engine().is_none());
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::Pending {
            control_failure: Some(ControlError::Deadline)
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
            control_failure: ControlError::Deadline
        }
    );
    assert_eq!(
        std::fs::read(dir.path().join("slot")).unwrap(),
        b"DGSL01C\n"
    );
    drop(control);
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    let mut parser = ControlReceiver::new_lifecycle([7; 32]);
    let frames = parser.push(&bytes).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].notification, ControlNotification::Ready {});
    assert_eq!(parser.end_of_stream(), Err(ControlError::Unconfirmed));
}

#[test]
fn lifecycle_disconnect_preserves_original_pending_and_records_unconfirmed_retirement() {
    let (dir, parts, registry) = crate::supervisor_binding_tests::fixture();
    let original = registry.reserve().unwrap();
    let owner =
        SupervisorOwner::bind(parts, deadline()).unwrap_or_else(|_| panic!("original owner"));
    let (sender, receiver) = UnixStream::pair().unwrap();
    let mut control = SupervisorControlOwner::new(
        owner,
        UnixControlWriter::new_lifecycle(sender, [7; 32]).unwrap(),
    );
    let cancel = AtomicBool::new(false);
    drop(receiver);
    assert_eq!(
        control.ready_lifecycle(&cancel, deadline()),
        Err(ControlError::Unconfirmed)
    );
    assert_eq!(
        control.foreground_ended_lifecycle(1, false, &cancel, deadline()),
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
fn lifecycle_mode_cannot_be_silently_selected_by_legacy_owner_methods() {
    let (_dir, parts, _registry) = crate::supervisor_binding_tests::fixture();
    let owner =
        SupervisorOwner::bind(parts, deadline()).unwrap_or_else(|_| panic!("original owner"));
    let (sender, _receiver) = UnixStream::pair().unwrap();
    let mut control = SupervisorControlOwner::new(
        owner,
        UnixControlWriter::new_lifecycle(sender, [7; 32]).unwrap(),
    );
    let cancel = AtomicBool::new(false);
    assert_eq!(control.ready(&cancel), Err(ControlError::Protocol));
    assert_eq!(
        control.ready_lifecycle(&cancel, deadline()),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        control.foreground_ended_lifecycle(1, false, &cancel, deadline()),
        Err(ControlError::Protocol)
    );
    assert_eq!(
        control.poll_retirement(deadline(), &cancel).unwrap(),
        SupervisorControlProgress::RetiredUnconfirmed {
            control_failure: ControlError::Protocol
        }
    );
}
