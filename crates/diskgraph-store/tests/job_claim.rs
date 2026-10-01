use diskgraph_core::{Locator, PrincipalId};
use diskgraph_store::{ControlStore, JobKind};

#[test]
fn concurrent_connections_cannot_both_claim_a_job() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut store = ControlStore::open(&path).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let job = store
        .create_job(
            &scope,
            JobKind::Index,
            &PrincipalId::new("requester").unwrap(),
        )
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let workers = (0..2)
        .map(|index| {
            let path = path.clone();
            let job = job.job_id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = ControlStore::open(&path).unwrap();
                barrier.wait();
                store.claim_job(&job, &format!("worker-{index}")).is_ok()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|claimed| *claimed)
            .count(),
        1
    );
}

#[test]
fn an_expired_job_is_reclaimed_instead_of_remaining_stuck() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut store = ControlStore::open(&path).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let job = store
        .create_job(
            &scope,
            JobKind::Index,
            &PrincipalId::new("requester").unwrap(),
        )
        .unwrap();
    store.claim_job(&job.job_id, "expired").unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE jobs SET heartbeat_unix_ms = 0, lease_expires_unix_ms = 0 WHERE job_id = ?1",
            [&job.job_id],
        )
        .unwrap();
    let claimed = store.claim_job(&job.job_id, "replacement").unwrap();
    assert_eq!(claimed.owner, "replacement");
    assert!(store.heartbeat(&job.job_id, "expired").is_err());
}

#[test]
fn an_expired_owner_cannot_finish_a_job() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut store = ControlStore::open(&path).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let job = store
        .create_job(
            &scope,
            JobKind::Index,
            &PrincipalId::new("requester").unwrap(),
        )
        .unwrap();
    store.claim_job(&job.job_id, "expired").unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms = 0 WHERE job_id = ?1",
            [&job.job_id],
        )
        .unwrap();
    assert!(
        store
            .finish_job(&job.job_id, "expired", diskgraph_store::JobState::Completed)
            .is_err()
    );
}

#[test]
fn a_reused_owner_name_does_not_reuse_the_old_fence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut store = ControlStore::open(&path).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let job = store
        .create_job(
            &scope,
            JobKind::Index,
            &PrincipalId::new("requester").unwrap(),
        )
        .unwrap();
    let first = store.claim_job(&job.job_id, "same-owner").unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms = 0 WHERE job_id = ?1",
            [&job.job_id],
        )
        .unwrap();
    let second = store.claim_job(&job.job_id, "same-owner").unwrap();
    assert!(second.fencing_token > first.fencing_token);
    assert!(
        store
            .with_job_fence(&job.job_id, "same-owner", first.fencing_token, || Ok(()))
            .is_err()
    );
    assert!(
        store
            .finish_job_fenced(
                &job.job_id,
                "same-owner",
                first.fencing_token,
                diskgraph_store::JobState::Completed
            )
            .is_err()
    );
    store
        .finish_job_fenced(
            &job.job_id,
            "same-owner",
            second.fencing_token,
            diskgraph_store::JobState::Completed,
        )
        .unwrap();
}

#[test]
fn a_revoked_policy_prevents_fenced_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut store = ControlStore::open(&path).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let job = store
        .create_job(
            &scope,
            JobKind::Index,
            &PrincipalId::new("requester").unwrap(),
        )
        .unwrap();
    let claimed = store.claim_job(&job.job_id, "owner").unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("INSERT INTO policy VALUES (1,1,1)", [])
        .unwrap();
    let mut called = false;
    assert!(
        store
            .with_job_fence(&job.job_id, "owner", claimed.fencing_token, || {
                called = true;
                Ok(())
            })
            .is_err()
    );
    assert!(!called);
}

#[test]
fn process_claim_worker() {
    let Some(base) = std::env::var_os("DG_CLAIM_FIXTURE") else {
        return;
    };
    let base = std::path::PathBuf::from(base);
    let owner = std::env::var("DG_CLAIM_OWNER").unwrap();
    let job = std::env::var("DG_CLAIM_JOB").unwrap();
    let mut store = ControlStore::open(&base.join("control.sqlite")).unwrap();
    std::fs::write(base.join(format!("ready-{owner}")), []).unwrap();
    let start = std::time::Instant::now();
    while !base.join("start").exists() {
        assert!(start.elapsed().as_secs() < 10);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    std::fs::write(
        base.join(format!("result-{owner}")),
        if store.claim_job_once(&job, &owner).is_ok() {
            "claimed"
        } else {
            "refused"
        },
    )
    .unwrap();
}

#[test]
fn two_processes_cannot_both_claim_a_job() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut store = ControlStore::open(&path).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let job = store
        .create_job(
            &scope,
            JobKind::Index,
            &PrincipalId::new("requester").unwrap(),
        )
        .unwrap();
    let mut children: Vec<_> = ["one", "two"]
        .iter()
        .map(|owner| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_claim_worker", "--nocapture"])
                .env("DG_CLAIM_FIXTURE", dir.path())
                .env("DG_CLAIM_OWNER", owner)
                .env("DG_CLAIM_JOB", &job.job_id)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let start = std::time::Instant::now();
    while !["one", "two"]
        .iter()
        .all(|owner| dir.path().join(format!("ready-{owner}")).exists())
    {
        assert!(start.elapsed().as_secs() < 10);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    std::fs::write(dir.path().join("start"), []).unwrap();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(
        ["one", "two"]
            .iter()
            .filter(
                |owner| std::fs::read_to_string(dir.path().join(format!("result-{owner}")))
                    .unwrap()
                    == "claimed"
            )
            .count(),
        1
    );
}

#[test]
fn merging_jobs_never_substitutes_another_request_subject() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ControlStore::open(&dir.path().join("control.sqlite")).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let alice = store
        .create_job(&scope, JobKind::Index, &PrincipalId::new("alice").unwrap())
        .unwrap();
    let bob = store
        .create_job(&scope, JobKind::Index, &PrincipalId::new("bob").unwrap())
        .unwrap();
    assert_ne!(alice.job_id, bob.job_id);
    assert_eq!(bob.principal.as_str(), "bob");
}

#[test]
fn admission_rechecks_the_live_grant_inside_its_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ControlStore::open(&dir.path().join("control.sqlite")).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    store.publish_policy_version(1).unwrap();
    assert!(
        store
            .create_job_with_quota(
                &scope,
                JobKind::Index,
                &PrincipalId::new("unauthorized").unwrap(),
                8
            )
            .is_err()
    );
    assert!(store.list_queued_jobs().unwrap().is_empty());
}

#[test]
fn migration_preserves_a_live_legacy_running_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut store = ControlStore::open(&path).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let job = store
        .create_job(
            &scope,
            JobKind::Index,
            &PrincipalId::new("requester").unwrap(),
        )
        .unwrap();
    let claimed = store.claim_job(&job.job_id, "old-owner").unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("ALTER TABLE jobs DROP COLUMN cancel_requested; ALTER TABLE jobs DROP COLUMN lease_expires_unix_ms; ALTER TABLE jobs DROP COLUMN fencing_token; PRAGMA user_version=3;").unwrap();
    drop(connection);
    let mut upgraded = ControlStore::open(&path).unwrap();
    let migrated = upgraded.job(&job.job_id).unwrap();
    assert_eq!(migrated.owner, "old-owner");
    assert_eq!(
        migrated.lease_expires_unix_ms,
        claimed.heartbeat_unix_ms + 30000
    );
    assert!(upgraded.claim_job(&job.job_id, "new-owner").is_err());
    assert!(
        dir.path()
            .join("migration_backups/control.sqlite.pre-v4.bak")
            .exists()
    );
}

#[test]
fn strict_claim_refuses_a_second_claim_by_the_same_owner() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ControlStore::open(&dir.path().join("control.sqlite")).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let job = store
        .create_job(
            &scope,
            JobKind::Index,
            &PrincipalId::new("requester").unwrap(),
        )
        .unwrap();
    store.claim_job_once(&job.job_id, "same-owner").unwrap();
    assert!(store.claim_job_once(&job.job_id, "same-owner").is_err());
}
