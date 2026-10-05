use crate::Engine;
use diskgraph_core::JobRequestAuthority;
use diskgraph_store::{JobRecord, JobState};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// 显式播种原认证已经到期的历史 Running 状态；来源：Rust D42 fence 优先级测试。
/// 只插入独占测试库，不绕过生产认领，也不把此记录称为本次生产认领的结果。
pub(super) struct HistoricalRunningSeed {
    pub(super) authority: JobRequestAuthority,
    pub(super) job: JobRecord,
}

impl HistoricalRunningSeed {
    /// 参数：已打开引擎、独占控制库、真实认领正控及其原身份；返回：另一个不可变的过期历史记录。
    pub(super) fn new(
        engine: &Engine,
        control_path: &Path,
        live_job: &JobRecord,
        live_authority: &JobRequestAuthority,
    ) -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let expiry = now.as_secs().checked_sub(1).unwrap();
        let now_ms = i64::try_from(now.as_millis()).unwrap();
        let authority = JobRequestAuthority::authenticated_remote(
            live_authority.principal().clone(),
            live_authority.issuer().unwrap(),
            live_authority.transport(),
            live_authority.capabilities().unwrap().to_vec(),
            expiry,
        )
        .unwrap();
        assert!(authority.validate_at(now.as_secs()).is_err());
        let job_id = format!("{}-historical-expired", live_job.job_id);
        let mut connection = Connection::open_with_flags(
            control_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        // 历史 material 只新增三行；不改已真实认领的正控，不删除不可变触发器或重写原认证。
        assert_eq!(
            tx.execute(
                "INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,
                 owner,principal,fencing_token,lease_expires_unix_ms,cancel_requested)
                 SELECT ?1,scope_id,kind,'running',?2,?2,'historical-owner',principal,1,?3,0
                 FROM jobs WHERE job_id=?4 AND state='running' AND kind='process_evidence'",
                params![
                    job_id,
                    now_ms,
                    now_ms.checked_add(30_000).unwrap(),
                    live_job.job_id
                ],
            )
            .unwrap(),
            1
        );
        assert_eq!(
            tx.execute(
                "INSERT INTO job_request_authorities(job_id,schema_version,authority_json) VALUES(?1,1,?2)",
                params![job_id, serde_json::to_string(&authority).unwrap()],
            )
            .unwrap(),
            1
        );
        assert_eq!(
            tx.execute(
                "INSERT INTO process_evidence_job_inputs(job_id,schema_version,input_json,input_sha256)
                 SELECT ?1,schema_version,input_json,input_sha256 FROM process_evidence_job_inputs WHERE job_id=?2",
                params![job_id, live_job.job_id],
            )
            .unwrap(),
            1
        );
        tx.commit().unwrap();
        drop(connection);
        let control = engine.control_store().unwrap();
        let job = control.job(&job_id).unwrap();
        assert_eq!(job.state, JobState::Running);
        assert_eq!(job.owner, "historical-owner");
        assert_eq!(job.fencing_token, 1);
        assert_eq!(job.scope_id, live_job.scope_id);
        assert_eq!(job.principal, live_job.principal);
        assert_eq!(
            control.job_request_authority(&job_id).unwrap(),
            Some(authority.clone())
        );
        assert_eq!(
            control.process_evidence_job_input(&job_id).unwrap(),
            control
                .process_evidence_job_input(&live_job.job_id)
                .unwrap()
        );
        drop(control);
        Self { authority, job }
    }
}
