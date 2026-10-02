use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

use super::LegacyDeliveryRegistry;
use crate::legacy_delivery_error::LegacyDeliveryError;
use crate::legacy_frame::LegacyFrame;

fn charged(registry: &LegacyDeliveryRegistry) -> usize {
    registry.inner.lock().unwrap().reserved_bytes
}

#[test]
fn queued_and_in_flight_frames_share_byte_and_count_budgets() {
    let registry = LegacyDeliveryRegistry::with_limits(400, 200, 3);
    let (id, receiver) = registry.open(Some("alice".into())).unwrap();
    let mut first = registry.reserve(&id, Some("alice"), 100).unwrap();
    let second = registry.reserve(&id, Some("alice"), 100).unwrap();
    assert!(matches!(
        registry.reserve(&id, Some("alice"), 1),
        Err(LegacyDeliveryError::Backpressure)
    ));
    let wire = crate::legacy::message_event("{}");
    first.shrink(wire.len()).unwrap();
    let third = registry
        .reserve(&id, Some("alice"), 100 - wire.len())
        .unwrap();
    assert!(matches!(
        registry.reserve(&id, Some("alice"), 0),
        Err(LegacyDeliveryError::Backpressure)
    ));
    registry
        .enqueue(LegacyFrame {
            wire,
            authorization_generation: 1,
            _reservation: first,
        })
        .unwrap();
    let writing = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    // 已出队但还在写入的帧不能提前归还，第三个请求仍占用最后一个条数。
    assert!(matches!(
        registry.reserve(&id, Some("alice"), 1),
        Err(LegacyDeliveryError::Backpressure)
    ));
    assert_eq!(charged(&registry), 200);
    drop(writing);
    drop(second);
    drop(third);
    assert_eq!(charged(&registry), 0);
}

#[test]
fn global_bytes_and_principal_binding_cannot_be_bypassed_by_another_session() {
    let registry = LegacyDeliveryRegistry::with_limits(300, 300, 64);
    let (a, _a_receiver) = registry.open(Some("alice".into())).unwrap();
    let (b, _b_receiver) = registry.open(Some("bob".into())).unwrap();
    let mut first = registry.reserve(&a, Some("alice"), 150).unwrap();
    let second = registry.reserve(&b, Some("bob"), 150).unwrap();
    assert!(matches!(
        registry.reserve(&a, Some("bob"), 1),
        Err(LegacyDeliveryError::PrincipalMismatch)
    ));
    assert!(matches!(
        registry.reserve(&b, Some("bob"), 1),
        Err(LegacyDeliveryError::Backpressure)
    ));
    first.shrink(10).unwrap();
    let third = registry.reserve(&b, Some("bob"), 100).unwrap();
    assert_eq!(charged(&registry), 260);
    drop((first, second, third));
    assert_eq!(charged(&registry), 0);
}

#[test]
fn concurrent_admission_cannot_overbook_the_global_bytes() {
    let registry = LegacyDeliveryRegistry::with_limits(128, 128, 64);
    let (id, _receiver) = registry.open(None).unwrap();
    let barrier = Arc::new(Barrier::new(9));
    let (sender, receiver) = mpsc::channel();
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let registry = registry.clone();
            let id = id.clone();
            let barrier = Arc::clone(&barrier);
            let sender = sender.clone();
            std::thread::spawn(move || {
                barrier.wait();
                sender.send(registry.reserve(&id, None, 32).ok()).unwrap();
            })
        })
        .collect();
    barrier.wait();
    let reservations: Vec<_> = (0..8)
        .filter_map(|_| receiver.recv_timeout(Duration::from_secs(2)).unwrap())
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(reservations.len(), 4);
    assert_eq!(charged(&registry), 128);
    drop(reservations);
    assert_eq!(charged(&registry), 0);
}

#[test]
fn closing_a_receiver_releases_queued_frames_and_invalidates_pending_work() {
    let registry = LegacyDeliveryRegistry::with_limits(400, 400, 64);
    let (id, receiver) = registry.open(None).unwrap();
    let mut queued = registry.reserve(&id, None, 100).unwrap();
    let mut pending = registry.reserve(&id, None, 100).unwrap();
    let wire = crate::legacy::message_event("{}");
    queued.shrink(wire.len()).unwrap();
    registry
        .enqueue(LegacyFrame {
            wire,
            authorization_generation: 1,
            _reservation: queued,
        })
        .unwrap();
    drop(receiver);
    assert_eq!(charged(&registry), 100);
    assert_eq!(pending.shrink(10), Err(LegacyDeliveryError::UnknownSession));
    assert!(matches!(
        registry.reserve(&id, None, 1),
        Err(LegacyDeliveryError::UnknownSession)
    ));
    drop(pending);
    assert_eq!(charged(&registry), 0);
    assert!(registry.inner.lock().unwrap().sessions.is_empty());
}

#[test]
fn disconnected_send_releases_the_frame_without_lock_reentry_deadlock() {
    let registry = LegacyDeliveryRegistry::with_limits(400, 400, 64);
    let (id, _receiver) = registry.open(None).unwrap();
    let mut reservation = registry.reserve(&id, None, 100).unwrap();
    let (sender, receiver) = mpsc::sync_channel(64);
    registry
        .inner
        .lock()
        .unwrap()
        .sessions
        .get_mut(&id)
        .unwrap()
        .sender = sender;
    drop(receiver);
    let wire = crate::legacy::message_event("{}");
    reservation.shrink(wire.len()).unwrap();
    assert_eq!(
        registry.enqueue(LegacyFrame {
            wire,
            authorization_generation: 1,
            _reservation: reservation
        }),
        Err(LegacyDeliveryError::UnknownSession)
    );
    assert_eq!(charged(&registry), 0);
}

#[test]
fn oversized_or_expanding_reservations_never_consume_more_credit() {
    let registry = LegacyDeliveryRegistry::with_limits(300, 200, 64);
    let (id, _receiver) = registry.open(None).unwrap();
    assert!(matches!(
        registry.reserve(&id, None, 201),
        Err(LegacyDeliveryError::ResponseLimit)
    ));
    assert_eq!(charged(&registry), 0);
    let mut reservation = registry.reserve(&id, None, 100).unwrap();
    assert_eq!(
        reservation.shrink(101),
        Err(LegacyDeliveryError::ResponseLimit)
    );
    assert_eq!(charged(&registry), 100);
    drop(reservation);
    assert_eq!(charged(&registry), 0);
}
