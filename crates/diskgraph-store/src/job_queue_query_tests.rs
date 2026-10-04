use crate::job_authority_tests::{configure, principal};
use crate::{ControlStore, JobKind, StoreError};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn page_steps(store: &ControlStore, maximum: usize) -> (Vec<crate::JobRecord>, u64) {
    let steps = Arc::new(AtomicU64::new(0));
    let counter = steps.clone();
    store
        .connection
        .progress_handler(
            1,
            Some(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                false
            }),
        )
        .unwrap();
    let records = store.list_queued_jobs_limited(maximum).unwrap();
    store
        .connection
        .progress_handler(0, None::<fn() -> bool>)
        .unwrap();
    (records, steps.load(Ordering::SeqCst))
}

#[test]
fn queue_page_uses_active_order_index_and_does_not_decode_unselected_jobs() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let scope = configure(&mut store);
    store
        .connection
        .execute(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<20000)
         INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,principal)
         SELECT printf('job-%06d',x),?1,'index','queued',x,x,'alice' FROM n",
            [scope.as_str()],
        )
        .unwrap();
    // 页外损坏 kind 是真实负控制：只读必要页成功；选择该行时仍不得吞掉真实解码错误。
    store
        .connection
        .execute(
            "UPDATE jobs SET kind='invalid' WHERE job_id='job-020000'",
            [],
        )
        .unwrap();
    let (one, one_steps) = page_steps(&store, 1);
    let (five, five_steps) = page_steps(&store, 5);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].job_id, "job-000001");
    assert_eq!(five.len(), 5);
    assert_eq!(five[4].job_id, "job-000005");
    println!("actual queue VM steps: page1={one_steps} page5={five_steps} total=20000");
    assert!(
        one_steps < 400,
        "page1 performed full queue work: {one_steps}"
    );
    assert!(
        five_steps < 1600,
        "page5 performed full queue work: {five_steps}"
    );
    assert!(five_steps > one_steps);
    assert!(store.list_queued_jobs_limited(0).unwrap().is_empty());
    assert!(matches!(
        store.list_queued_jobs_limited(20000),
        Err(StoreError::Sqlite(_))
    ));
}

#[test]
fn manual_strict_reap_is_limited_and_does_not_preempt_a_live_legacy_lease() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let scope = configure(&mut store);
    let live = store
        .create_job(&scope, JobKind::Index, &principal())
        .unwrap();
    let live = store.claim_job_once(&live.job_id, "old-owner").unwrap();
    store
        .connection
        .execute(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<65)
         INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,principal)
         SELECT printf('queued-%03d',x),?1,'index','queued',x,x,'legacy-subject' FROM n",
            [scope.as_str()],
        )
        .unwrap();
    assert_eq!(store.reap_request_jobs_strict().unwrap(), 64);
    assert_eq!(store.job(&live.job_id).unwrap(), live);
    assert_eq!(store.list_queued_jobs_limited(64).unwrap().len(), 1);
    assert_eq!(store.reap_request_jobs_strict().unwrap(), 1);
    assert!(store.list_queued_jobs_limited(64).unwrap().is_empty());
    assert_eq!(store.job(&live.job_id).unwrap(), live);
}
