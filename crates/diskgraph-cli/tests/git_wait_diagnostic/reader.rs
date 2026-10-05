//! Git wait 失败后的隔离只读诊断；来源：原生 Rust CLI 集成测试。

use crate::GitFixture;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::path::Path;

/// 失败 CLI 退出后的窄只读诊断；来源：原生 Rust 集成测试，不重新执行采集。
/// 两库读取并非原子快照，也不证明原请求曾到达任何内部阶段。
pub(crate) fn read(fixture: &GitFixture) -> Value {
    let read = || -> Result<Value, &'static str> {
        let control =
            diagnostic_database(&fixture.temp.path().join("data/diskgraph-control.sqlite"))?;
        let graph = diagnostic_database(&fixture.temp.path().join("data/diskgraph.sqlite"))?;
        let job = control.query_row(
            "SELECT j.job_id, CASE WHEN j.state IN ('queued','running','completed','failed','cancelled')
             THEN j.state ELSE 'invalid' END, j.fencing_token,
             EXISTS(SELECT 1 FROM git_evidence_job_inputs i WHERE i.job_id=j.job_id),
             CASE WHEN length(CAST(f.phase AS BLOB))<=32 THEN f.phase ELSE NULL END,
             CASE WHEN length(CAST(f.code AS BLOB))<=64 THEN f.code ELSE NULL END
             FROM jobs j LEFT JOIN git_job_failures f ON f.job_id=j.job_id
             WHERE j.scope_id=?1 AND j.kind='git_evidence'
             ORDER BY j.created_at_unix_ms DESC,j.job_id DESC LIMIT 1",
            [&fixture.scope], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?, row.get::<_, bool>(3)?,
                row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?)),
        ).optional().map_err(|_| "control_job_read_failed")?;
        let mut state = Value::Null;
        let mut fence = Value::Null;
        let mut typed_input = Value::Null;
        let mut failure = Value::Null;
        let mut receipt = Value::Null;
        if let Some((job_id, job_state, job_fence, input_exists, phase, code)) = &job {
            state = serde_json::json!(job_state);
            let validated_fence =
                u64::try_from(*job_fence).map_err(|_| "invalid_persisted_fence")?;
            fence = serde_json::json!(validated_fence);
            typed_input = serde_json::json!(input_exists);
            if phase.is_some() || code.is_some() {
                let typed: diskgraph_core::GitEvidenceFailure =
                    serde_json::from_value(serde_json::json!({"phase":phase,"code":code}))
                        .map_err(|_| "invalid_persisted_safe_failure")?;
                failure = serde_json::json!(typed);
            }
            let exists: bool = graph
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM job_publication_receipts WHERE job_id=?1)",
                    [job_id],
                    |row| row.get(0),
                )
                .map_err(|_| "graph_receipt_read_failed")?;
            receipt = serde_json::json!(exists);
        }
        // 仅比较实际 base owner 的 latest，不输出 server/scope/revision 原始标识。
        let latest_is_base: Option<bool> = graph
            .query_row(
                "SELECT r.revision_id=?1 FROM graph_revisions r JOIN revision_ownership o
             ON o.revision_id=r.revision_id WHERE o.scope_id=?2 AND o.server_id=
             (SELECT server_id FROM revision_ownership WHERE revision_id=?1 AND scope_id=?2)
             ORDER BY r.published_at_unix_ms DESC,r.revision_id DESC LIMIT 1",
                rusqlite::params![fixture.revision, fixture.scope],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "graph_latest_read_failed")?;
        let diagnostic = serde_json::json!({"observation":"posterior_non_atomic","read_status":"ok",
            "git_job_exists":job.is_some(),"state":state,"fence":fence,
            "typed_input_exists":typed_input,"safe_failure":failure,"receipt_exists":receipt,
            "latest_exists":latest_is_base.is_some(),"latest_matches_base":latest_is_base});
        Ok(diagnostic)
    };
    read().unwrap_or_else(
        |code| serde_json::json!({"observation":"posterior_non_atomic","read_status":code}),
    )
}

/// 参数：隔离夹具现有库；返回：只读连接或固定诊断码，不迁移、不输出路径。
fn diagnostic_database(path: &Path) -> Result<Connection, &'static str> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "diagnostic_database_open_failed")?;
    connection
        .busy_timeout(std::time::Duration::from_millis(100))
        .map_err(|_| "diagnostic_read_timeout_configuration_failed")?;
    Ok(connection)
}
