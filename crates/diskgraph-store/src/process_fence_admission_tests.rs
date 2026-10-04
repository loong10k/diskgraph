//! 真实认领/fence 与会话准入的错误优先级；不证明跨库原子或原生平台能力。
use crate::process_job_test_fixtures::fixture;
use crate::{ControlStore, JobRecord, Result, StoreError};
use diskgraph_core::{JobRequestAuthority, Permission};
use std::time::{Duration, Instant};

fn running() -> (ControlStore, JobRecord) {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let running = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    (control, running)
}
fn stopped(_: u64, _: u64, _: u64) -> Result<()> {
    Err(StoreError::BudgetExceeded)
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[test]
fn process_fence_admission_uses_the_actual_transaction_lease_and_same_costs() {
    let (mut control, job) = running();
    control
        .connection
        .busy_timeout(Duration::from_millis(731))
        .unwrap();
    let before = control.job(&job.job_id).unwrap();
    let mut costs = (0_u64, 0_u64, 0_u64);
    let value = control
        .with_job_fence_with_admission(
            &job.job_id,
            "owner",
            job.fencing_token,
            deadline(),
            &mut |raw, entries, allocation| {
                costs.0 += raw;
                costs.1 += entries;
                costs.2 += allocation;
                Ok(())
            },
            Ok,
        )
        .unwrap();
    assert_eq!(value, job.lease_expires_unix_ms);
    assert!(costs.0 > 100 && costs.1 >= 5 && costs.2 > costs.0);
    assert_eq!(control.job(&job.job_id).unwrap(), before);
    assert!(control.connection.is_autocommit());
    let busy: i64 = control
        .connection
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(busy, 731);
}

#[test]
fn process_fence_admission_stopped_session_never_enters_work_or_changes_job() {
    let (mut control, job) = running();
    let mut called = false;
    let result = control.with_job_fence_with_admission(
        &job.job_id,
        "owner",
        job.fencing_token,
        deadline(),
        &mut stopped,
        |_| {
            called = true;
            Ok(())
        },
    );
    assert!(matches!(result, Err(StoreError::BudgetExceeded)));
    assert!(!called);
    assert_eq!(control.job(&job.job_id).unwrap(), job);
    assert!(control.connection.is_autocommit());
    assert!(
        control
            .with_job_fence(&job.job_id, "owner", job.fencing_token, || Ok(()))
            .is_ok()
    );
}

#[test]
fn process_fence_admission_live_revoke_cancel_and_owner_refusal_precede_session() {
    for denial in ["metadata", "index", "cancel", "owner"] {
        let (mut control, job) = running();
        match denial {
            "metadata" => assert_eq!(
                control
                    .connection
                    .execute(
                        "DELETE FROM grants WHERE permission=?1",
                        [Permission::MetadataRead.wire_name()]
                    )
                    .unwrap(),
                1
            ),
            "index" => assert_eq!(
                control
                    .connection
                    .execute(
                        "DELETE FROM grants WHERE permission=?1",
                        [Permission::IndexWrite.wire_name()]
                    )
                    .unwrap(),
                1
            ),
            "cancel" => {
                control.request_cancel(&job.job_id).unwrap();
            }
            "owner" => {}
            _ => unreachable!(),
        }
        let mut admitted = false;
        let mut worked = false;
        let result = control.with_job_fence_with_admission(
            &job.job_id,
            if denial == "owner" {
                "foreign"
            } else {
                "owner"
            },
            job.fencing_token,
            deadline(),
            &mut |_, _, _| {
                admitted = true;
                Err(StoreError::BudgetExceeded)
            },
            |_| {
                worked = true;
                Ok(())
            },
        );
        if denial == "metadata" {
            assert!(
                matches!(result,Err(StoreError::Conflict(ref message)) if message=="live job authorization withdrawn")
            );
        } else {
            assert!(matches!(result, Err(StoreError::StaleOwner)));
        }
        assert!(
            !admitted && !worked,
            "{denial} did not win before a stopped session"
        );
        assert!(control.connection.is_autocommit());
    }
}

#[test]
fn process_fence_admission_expired_original_identity_precedes_session_failure() {
    let (mut control, job) = running();
    let current = control.job_request_authority(&job.job_id).unwrap().unwrap();
    let expired = JobRequestAuthority::authenticated_remote(
        current.principal().clone(),
        "issuer",
        "http",
        vec![Permission::MetadataRead, Permission::IndexWrite],
        ControlStore::now_ms() / 1000,
    )
    .unwrap();
    // 显式损坏外连边界：不绕生产 immutable API，仅模拟持久到期这一安全判断。
    control
        .connection
        .execute_batch("DROP TRIGGER job_authority_immutable_update;")
        .unwrap();
    control
        .connection
        .execute(
            "UPDATE job_request_authorities SET authority_json=?1 WHERE job_id=?2",
            rusqlite::params![serde_json::to_string(&expired).unwrap(), job.job_id],
        )
        .unwrap();
    let mut admitted = false;
    let result = control.with_job_fence_with_admission(
        &job.job_id,
        "owner",
        job.fencing_token,
        deadline(),
        &mut |_, _, _| {
            admitted = true;
            Err(StoreError::BudgetExceeded)
        },
        |_| Ok(()),
    );
    assert!(
        matches!(result,Err(StoreError::Conflict(ref message)) if message=="job request authority denied")
    );
    assert!(!admitted);
    assert!(control.connection.is_autocommit());
}

#[test]
fn process_fence_admission_keeps_original_deadline_and_rolls_back_failed_work() {
    let (mut control, job) = running();
    let before = control.job(&job.job_id).unwrap();
    let expired = Instant::now() - Duration::from_millis(1);
    assert!(matches!(
        control.with_job_fence_with_admission(
            &job.job_id,
            "owner",
            job.fencing_token,
            expired,
            &mut |_, _, _| Ok(()),
            |_| Ok(())
        ),
        Err(StoreError::BudgetExceeded)
    ));
    let failure = control.with_job_fence_with_admission(
        &job.job_id,
        "owner",
        job.fencing_token,
        deadline(),
        &mut |_, _, _| Ok(()),
        |_| Err::<(), _>(StoreError::Conflict("publication refused".into())),
    );
    assert!(
        matches!(failure,Err(StoreError::Conflict(ref message)) if message=="publication refused")
    );
    assert_eq!(control.job(&job.job_id).unwrap(), before);
    assert!(control.connection.is_autocommit());
    assert!(
        control
            .with_job_fence(&job.job_id, "owner", job.fencing_token, || Ok(()))
            .is_ok()
    );
}
