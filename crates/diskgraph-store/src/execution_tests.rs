use crate::execution_codec::{action_from_name, action_name, claim_keys, now_ms};
use crate::{
    Approval, IntentState, Operation, OperationItemResult, OperationState, Plan, PlanItem,
    PlanState, RecoveryEntry, RecoveryRule, RecoveryState, StoreError,
};
use diskgraph_core::{FileActionKind, PrincipalId, ScopeId};

fn plan(plan_id: &str) -> Plan {
    Plan {
        plan_id: plan_id.into(),
        scope_id: ScopeId::new("scope-1").unwrap(),
        principal: PrincipalId::new("agent").unwrap(),
        action: FileActionKind::Trash,
        items: vec![PlanItem {
            node_id: 1,
            locator_key: "raw-key-1".into(),
            identity: Some("dev:ino".into()),
            source_fingerprint: None,
            includes_descendants: false,
            recovery_ref: None,
        }],
        target_locator_key: None,
        policy_version: 1,
        max_bytes: 1024,
        created_at_unix_ms: now_ms(),
        expires_at_unix_ms: now_ms() + 60_000,
        recovery: RecoveryRule::Quarantine,
        expected_bytes: 512,
    }
}

fn operation(id: &str, key: &str, digest: &str) -> Operation {
    Operation {
        operation_id: id.into(),
        plan_id: "plan-1".into(),
        scope_id: ScopeId::new("scope-1").unwrap(),
        principal: PrincipalId::new("agent").unwrap(),
        idempotency_key: key.into(),
        request_digest: digest.into(),
        state: OperationState::Queued,
        created_at_unix_ms: now_ms(),
        updated_at_unix_ms: now_ms(),
    }
}

fn approval(ref_id: &str, digest: &str) -> Approval {
    Approval {
        approval_ref: ref_id.into(),
        plan_id: "plan-1".into(),
        plan_digest: digest.into(),
        principal: PrincipalId::new("agent").unwrap(),
        action: FileActionKind::Trash,
        issued_by: "admin-console".into(),
        issued_at_unix_ms: now_ms(),
        expires_at_unix_ms: now_ms() + 60_000,
        revoked: false,
    }
}

/// The scope every fixture plan and operation belongs to. A fixed id
/// keeps the foreign keys satisfied without a lookup per test.
const FIXTURE_SCOPE: &str = "scope-1";

/// Opens a store with the fixture scope registered, so plan and operation
/// rows satisfy their foreign keys the way they do in production.
fn scoped_store(_label: &str) -> crate::ControlStore {
    let mut store = crate::ControlStore::open_in_memory().unwrap();
    // Drive the scope id directly so fixtures can reference it.
    store
        .insert_scope_row(
            FIXTURE_SCOPE,
            &diskgraph_core::Locator::from_native_path(std::path::Path::new("/tmp/fixture")),
        )
        .unwrap();
    store
}

#[test]
fn plans_are_immutable_and_digest_addressed() {
    let mut store = scoped_store("plans");
    let plan = plan("plan-1");
    store.insert_plan(&plan, "digest-1").unwrap();
    assert_eq!(store.plan("plan-1").unwrap(), plan);
    assert_eq!(store.plan_digest("plan-1").unwrap(), "digest-1");
    assert!(matches!(
        store.insert_plan(&plan, "digest-2"),
        Err(StoreError::Sqlite(_))
    ));
    // A plan can only be consumed once.
    assert_eq!(store.plan_state("plan-1").unwrap(), PlanState::Validated);
    store.mark_plan_applied("plan-1").unwrap();
    assert_eq!(store.plan_state("plan-1").unwrap(), PlanState::Applied);
    assert!(store.mark_plan_applied("plan-1").is_err());
}

#[test]
fn expired_plans_never_look_valid() {
    let mut store = scoped_store("expired");
    let mut plan = plan("plan-expired");
    plan.expires_at_unix_ms = now_ms() - 1;
    store.insert_plan(&plan, "d").unwrap();
    assert_eq!(
        store.plan_state("plan-expired").unwrap(),
        PlanState::Expired
    );
}

#[test]
fn approvals_bind_to_digest_principal_and_action() {
    let mut store = scoped_store("approvals");
    store.insert_plan(&plan("plan-1"), "digest-1").unwrap();
    store
        .insert_approval(&approval("ap-1", "digest-1"))
        .unwrap();
    let principal = PrincipalId::new("agent").unwrap();
    assert!(
        store
            .verify_approval(
                "ap-1",
                "plan-1",
                "digest-1",
                &principal,
                FileActionKind::Trash
            )
            .is_ok()
    );
    // Wrong plan digest, principal, and action are all refused.
    assert!(
        store
            .verify_approval(
                "ap-1",
                "plan-1",
                "digest-other",
                &principal,
                FileActionKind::Trash
            )
            .is_err()
    );
    assert!(
        store
            .verify_approval(
                "ap-1",
                "plan-other",
                "digest-1",
                &principal,
                FileActionKind::Trash
            )
            .is_err()
    );
    let other = PrincipalId::new("intruder").unwrap();
    assert!(
        store
            .verify_approval("ap-1", "plan-1", "digest-1", &other, FileActionKind::Trash)
            .is_err()
    );
    assert!(
        store
            .verify_approval(
                "ap-1",
                "plan-1",
                "digest-1",
                &principal,
                FileActionKind::Move
            )
            .is_err()
    );
}

#[test]
fn revoked_and_expired_approvals_are_refused() {
    let mut store = scoped_store("revoked");
    store.insert_plan(&plan("plan-1"), "digest-1").unwrap();
    store
        .insert_approval(&approval("ap-1", "digest-1"))
        .unwrap();
    store.revoke_approval("ap-1").unwrap();
    let principal = PrincipalId::new("agent").unwrap();
    assert!(
        store
            .verify_approval(
                "ap-1",
                "plan-1",
                "digest-1",
                &principal,
                FileActionKind::Trash
            )
            .is_err()
    );

    let mut expired = approval("ap-2", "digest-1");
    expired.expires_at_unix_ms = now_ms() - 1;
    store.insert_approval(&expired).unwrap();
    assert!(
        store
            .verify_approval(
                "ap-2",
                "plan-1",
                "digest-1",
                &principal,
                FileActionKind::Trash
            )
            .is_err()
    );
}

#[test]
fn one_idempotency_key_maps_to_one_operation() {
    let mut store = scoped_store("idempotency");
    store.insert_plan(&plan("plan-1"), "plan-digest").unwrap();

    let (id, created) = store
        .begin_operation(&operation("op-1", "key-1", "d-1"), 2)
        .unwrap();
    assert!(created);
    assert_eq!(id, "op-1");
    // Same request, same key: the original operation is returned.
    let (again, created) = store
        .begin_operation(&operation("op-2", "key-1", "d-1"), 2)
        .unwrap();
    assert!(!created);
    assert_eq!(again, "op-1");
    // Same key, different request: refused, never merged.
    assert!(matches!(
        store.begin_operation(&operation("op-3", "key-1", "d-2"), 2),
        Err(StoreError::IdempotencyConflict)
    ));

    // A different principal may reuse the key, in its own scope.
    store
        .insert_scope_row(
            "scope-2",
            &diskgraph_core::Locator::from_native_path(std::path::Path::new("/tmp/other")),
        )
        .unwrap();
    store
        .insert_plan(
            &Plan {
                scope_id: ScopeId::new("scope-2").unwrap(),
                principal: PrincipalId::new("other").unwrap(),
                items: vec![PlanItem {
                    locator_key: "other-resource".into(),
                    identity: Some("other-dev:ino".into()),
                    ..plan("unused").items[0].clone()
                }],
                ..plan("plan-2")
            },
            "plan-digest-2",
        )
        .unwrap();
    let mut other = operation("op-4", "key-1", "d-1");
    other.principal = PrincipalId::new("other").unwrap();
    other.scope_id = ScopeId::new("scope-2").unwrap();
    other.plan_id = "plan-2".into();
    let (_, created) = store.begin_operation(&other, 1).unwrap();
    assert!(created);
}

#[test]
fn overlapping_sources_and_targets_are_claimed_in_one_transaction() {
    let mut store = scoped_store("atomic-claims");
    let mut first = plan("plan-1");
    first.items[0].locator_key = "2f746d702f736f75726365".into(); // /tmp/source
    first.target_locator_key = Some("2f746d702f746172676574".into()); // /tmp/target
    store.insert_plan(&first, "first").unwrap();
    store
        .begin_operation(&operation("op-1", "claim-a", "digest-a"), 1)
        .unwrap();

    let mut second = plan("plan-2");
    second.items[0].locator_key = "2f746d702f736f757263652f6368696c64".into(); // descendant
    store.insert_plan(&second, "second").unwrap();
    let mut second_op = operation("op-2", "claim-b", "digest-b");
    second_op.plan_id = second.plan_id.clone();
    assert!(matches!(
        store.begin_operation(&second_op, 1),
        Err(StoreError::Conflict(_))
    ));

    let mut third = plan("plan-3");
    third.items[0].locator_key = "2f746d702f6f746865722f736f75726365".into(); // /tmp/other/source
    third.target_locator_key = first.target_locator_key.clone();
    store.insert_plan(&third, "third").unwrap();
    let mut third_op = operation("op-3", "claim-c", "digest-c");
    third_op.plan_id = third.plan_id.clone();
    assert!(matches!(
        store.begin_operation(&third_op, 1),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(store.operation_items("op-1").unwrap().len(), 1);
    assert!(matches!(
        store.operation("op-2"),
        Err(StoreError::OperationNotFound(_))
    ));
}

#[test]
fn hardlink_aliases_claim_the_same_source_identity() {
    let mut store = scoped_store("hardlink-claim");
    let mut first = plan("plan-1");
    first.items[0].locator_key = "2f746d702f6669727374".into();
    first.items[0].identity = Some("42:7".into());
    store.insert_plan(&first, "first").unwrap();
    store
        .begin_operation(&operation("op-1", "claim-first", "digest-first"), 1)
        .unwrap();

    let mut alias = plan("plan-2");
    alias.items[0].locator_key = "2f746d702f616c696173".into();
    alias.items[0].identity = Some("42:7".into());
    store.insert_plan(&alias, "alias").unwrap();
    let mut request = operation("op-2", "claim-alias", "digest-alias");
    request.plan_id = alias.plan_id;
    assert!(matches!(
        store.begin_operation(&request, 1),
        Err(StoreError::Conflict(_))
    ));
}

#[test]
fn encoded_separator_detection_stays_on_byte_boundaries() {
    let mut example = plan("hex-boundary");
    example.items[0].locator_key = "2f12f3".into(); // / followed by bytes 0x12, 0xf3
    example.target_locator_key = Some("2f746d70".into()); // /tmp
    assert_eq!(
        claim_keys(&example),
        vec![
            "2f12f3".to_string(),
            "identity:dev:ino".to_string(),
            "2f746d702f12f3".to_string()
        ]
    );
}

#[test]
fn independent_connections_cannot_claim_the_same_source() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("control.sqlite");
    {
        let mut store = crate::ControlStore::open(&path).unwrap();
        store
            .insert_scope_row(
                FIXTURE_SCOPE,
                &diskgraph_core::Locator::from_native_path(std::path::Path::new("/tmp/fixture")),
            )
            .unwrap();
        store.insert_plan(&plan("plan-1"), "first").unwrap();
        store.insert_plan(&plan("plan-2"), "second").unwrap();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|index| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = crate::ControlStore::open(&path).unwrap();
                let mut request = operation(
                    &format!("op-{index}"),
                    &format!("key-{index}"),
                    &format!("digest-{index}"),
                );
                request.plan_id = format!("plan-{}", index + 1);
                barrier.wait();
                store.begin_operation(&request, 1)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::Conflict(_))))
            .count(),
        1
    );
}

#[test]
fn intent_is_recorded_before_results() {
    let mut store = scoped_store("intent");
    store.insert_plan(&plan("plan-1"), "plan-digest").unwrap();
    store
        .begin_operation(&operation("op-1", "key-1", "d-1"), 2)
        .unwrap();
    let items = store.operation_items("op-1").unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|item| item.intent == IntentState::Pending));
    store.record_intent("op-1", 0).unwrap();
    store
        .record_item_result(
            "op-1",
            0,
            OperationItemResult::Quarantined,
            "ok",
            Some("rec-1"),
        )
        .unwrap();
    let items = store.operation_items("op-1").unwrap();
    assert_eq!(items[0].intent, IntentState::IntentRecorded);
    assert_eq!(items[0].result, OperationItemResult::Quarantined);
    assert_eq!(items[0].recovery_ref.as_deref(), Some("rec-1"));
    // The untouched item is still pending, so a crash mid-plan is visible.
    assert_eq!(items[1].intent, IntentState::Pending);
}

#[test]
fn recovery_entries_survive_and_can_be_consumed_once() {
    let mut store = scoped_store("recovery");
    let entry = RecoveryEntry {
        recovery_ref: "rec-1".into(),
        operation_id: "op-1".into(),
        scope_id: ScopeId::new("scope-1").unwrap(),
        original_locator: "raw-original".into(),
        quarantine_locator: "raw-quarantine".into(),
        identity: "dev:ino".into(),
        created_at_unix_ms: now_ms(),
        state: RecoveryState::Available,
    };
    store.insert_recovery(&entry).unwrap();
    assert_eq!(store.recovery("rec-1").unwrap(), entry);
    store.mark_recovery_restored("rec-1").unwrap();
    assert_eq!(
        store.recovery("rec-1").unwrap().state,
        RecoveryState::Restored
    );
}

#[test]
fn operations_are_listed_per_scope() {
    let mut store = scoped_store("listed");
    store.insert_plan(&plan("plan-1"), "plan-digest").unwrap();
    store
        .begin_operation(&operation("op-1", "k1", "d"), 1)
        .unwrap();
    store
        .insert_scope_row(
            "scope-2",
            &diskgraph_core::Locator::from_native_path(std::path::Path::new("/tmp/second")),
        )
        .unwrap();
    store
        .insert_plan(
            &Plan {
                scope_id: ScopeId::new("scope-2").unwrap(),
                items: vec![PlanItem {
                    locator_key: "another-resource".into(),
                    identity: Some("another-dev:ino".into()),
                    ..plan("unused").items[0].clone()
                }],
                ..plan("plan-2")
            },
            "plan-digest-2",
        )
        .unwrap();
    let mut other_scope = operation("op-2", "k2", "d");
    other_scope.scope_id = ScopeId::new("scope-2").unwrap();
    other_scope.plan_id = "plan-2".into();
    store.begin_operation(&other_scope, 1).unwrap();
    let listed = store
        .list_operations(&ScopeId::new("scope-1").unwrap(), 10)
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].operation_id, "op-1");
}

#[test]
fn action_names_round_trip() {
    for action in [
        FileActionKind::Move,
        FileActionKind::Copy,
        FileActionKind::Trash,
        FileActionKind::Restore,
        FileActionKind::Purge,
    ] {
        assert_eq!(action_from_name(action_name(action)), Some(action));
    }
}
