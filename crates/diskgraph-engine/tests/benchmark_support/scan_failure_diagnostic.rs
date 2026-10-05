//! 隔离成本夹具的失败后验；来源：原生 Rust 配对扫描验收，不属于执行或恢复协议。

use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use rusqlite::{Connection, OpenFlags, Params, Row, params};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 原执行返回失败后才读取固定标量的非原子诊断，不反推已经丢失的首个停止原因。
/// 来源：DiskGraph Linux 配对成本验收；无 Java 对应对象。
pub(crate) struct ScanFailureDiagnostic {
    report: Value,
}

impl ScanFailureDiagnostic {
    /// 参数：data 为既有数据库目录；job/scope/principal/owner 为原请求身份；error/elapsed 为原返回事实。
    /// 返回：两库独立只读后验；每库共享 50,000 VM 步和 100ms 协作上限，原执行期限不变。
    pub(crate) fn capture(
        data: &Path,
        job: &str,
        scope: &str,
        principal: &str,
        owner: &str,
        error: &EngineError,
        elapsed: Duration,
    ) -> Self {
        let started = Instant::now();
        let clock = SystemTime::now().duration_since(UNIX_EPOCH);
        let now = clock
            .as_ref()
            .ok()
            .and_then(|value| i64::try_from(value.as_millis()).ok());
        let mut control_report = json!({});
        let mut fence = None;
        Self::read_database(
            &data.join("diskgraph-control.sqlite"),
            &mut control_report,
            |connection, report| {
                let record = Self::query(
                    connection,
                    "SELECT CASE state WHEN 'queued' THEN 'queued' WHEN 'running' THEN 'running' WHEN 'completed' THEN 'completed' WHEN 'failed' THEN 'failed' WHEN 'cancelled' THEN 'cancelled' ELSE 'unknown' END,owner=?2,scope_id=?3,principal=?4,fencing_token,cancel_requested,heartbeat_unix_ms,lease_expires_unix_ms FROM jobs WHERE job_id=?1",
                    params![job, owner, scope, principal],
                    |row| {
                        let heartbeat: i64 = row.get(6)?;
                        let lease: i64 = row.get(7)?;
                        Ok(
                            json!({"state":row.get::<_,String>(0)?,"owner_matches_request":row.get::<_,bool>(1)?,"scope_matches_request":row.get::<_,bool>(2)?,"principal_matches_request":row.get::<_,bool>(3)?,"fencing_token":row.get::<_,i64>(4)?,"cancel_requested":row.get::<_,bool>(5)?,"heartbeat_unix_ms":heartbeat,"lease_expires_unix_ms":lease,"heartbeat_age_ms_at_diagnostic_start":now.map(|value|value.saturating_sub(heartbeat)),"lease_remaining_ms_at_diagnostic_start":now.map(|value|lease.saturating_sub(value))}),
                        )
                    },
                );
                fence = record["value"]["fencing_token"]
                    .as_i64()
                    .filter(|value| *value >= 0);
                report["job"] = record;
                report["scope"] = Self::query(
                    connection,
                    "SELECT revoked FROM scopes WHERE scope_id=?1",
                    [scope],
                    |row| Ok(json!({"revoked":row.get::<_,bool>(0)?})),
                );
                report["policy"] = Self::query(
                    connection,
                    "SELECT version,revoked FROM policy WHERE id=1",
                    [],
                    |row| {
                        Ok(json!({"version":row.get::<_,i64>(0)?,"revoked":row.get::<_,bool>(1)?}))
                    },
                );
                report["index_grant"] = Self::query(
                    connection,
                    "SELECT EXISTS(SELECT 1 FROM policy p JOIN grants g ON g.policy_version=p.version WHERE p.id=1 AND p.revoked=0 AND g.principal_id=?1 AND g.scope_id=?2 AND g.permission='index:write')",
                    params![principal, scope],
                    |row| Ok(json!({"present_for_live_policy":row.get::<_,bool>(0)?})),
                );
            },
        );
        let mut graph_report = json!({});
        Self::read_database(
            &data.join("diskgraph.sqlite"),
            &mut graph_report,
            |connection, report| {
                report["scope_latest_pointer"] = Self::query(
                    connection,
                    "SELECT EXISTS(SELECT 1 FROM latest_revision l JOIN revision_ownership o ON o.revision_id=l.revision_id WHERE o.scope_id=?1)",
                    [scope],
                    |row| Ok(json!({"exists":row.get::<_,bool>(0)?})),
                );
                if let Some(fence) = fence {
                    let namespace = format!("{job}:{fence}");
                    // 固定表与 LIMIT 限制计数；4097 表示下界，不冒充完整暂存表大小。
                    for (name, sql) in [
                        (
                            "nodes",
                            "SELECT count(*) FROM (SELECT 1 FROM scan_staging WHERE job_id=?1 LIMIT 4097)",
                        ),
                        (
                            "search",
                            "SELECT count(*) FROM (SELECT 1 FROM scan_staging_search WHERE job_id=?1 LIMIT 4097)",
                        ),
                        (
                            "unix_observations",
                            "SELECT count(*) FROM (SELECT 1 FROM scan_staging_unix_observations WHERE job_id=?1 LIMIT 4097)",
                        ),
                    ] {
                        report[name] = Self::query(connection, sql, [&namespace], |row| {
                            let count: i64 = row.get(0)?;
                            Ok(json!({"rows":count,"is_lower_bound":count==4097}))
                        });
                    }
                } else {
                    report["staging"] =
                        json!({"status":"not_attempted","reason":"actual_generation_unavailable"});
                }
            },
        );
        // 同源 harness 也覆盖没有 primary() API 的旧基线；只沿未知包装的 typed source 借用迭代。
        let mut primary_error = error;
        let original_error = loop {
            break match primary_error {
                EngineError::Business(error) => json!({"category":"business","code":error.code()}),
                EngineError::Store(StoreError::Sqlite(error)) => Self::sql_error(error),
                EngineError::Store(StoreError::StaleOwner) => {
                    json!({"category":"store","code":"stale_owner"})
                }
                EngineError::Store(StoreError::Conflict(_)) => {
                    json!({"category":"store","code":"conflict"})
                }
                EngineError::Store(StoreError::BudgetExceeded) => {
                    json!({"category":"store","code":"budget_exceeded"})
                }
                EngineError::Store(_) => json!({"category":"store","code":"other_store_error"}),
                EngineError::Io(error) => Self::io_error(error),
                other => {
                    if matches!(other, EngineError::Poisoned) {
                        json!({"category":"poisoned"})
                    } else if let Some(primary) = std::error::Error::source(other)
                        .and_then(|source| source.downcast_ref::<EngineError>())
                    {
                        primary_error = primary;
                        continue;
                    } else {
                        json!({"category":"unclassified_engine_error"})
                    }
                }
            };
        };
        Self {
            report: json!({"schema_version":1,"observation":"posterior_non_atomic","original_run_elapsed_seconds":elapsed.as_secs_f64(),"original_error":original_error,"observed_at_unix_ms":now,"wall_clock_available":now.is_some(),"diagnostic_elapsed_seconds":started.elapsed().as_secs_f64(),"limits":{"busy_timeout_ms":0,"vm_steps_per_database":50000,"cooperative_ms_per_database":100,"staging_count_cap":4097},"control":control_report,"graph":graph_report,"limitations":["Observed only after run_job returned; these reads are not an atomic snapshot or a witness of the first keeper/poll/native failure.","No Engine reopen, migration, recovery, rescan, write SQL, or retry; native database opening and I/O have no hard real-time guarantee.","Identity/path/body strings are not read or emitted; errors retain only fixed categories and numeric native codes."]}),
        }
    }

    /// 参数：无；使用已经完成的后验报告。返回：无，输出失败不能替换原 run_job 错误。
    /// 原 stderr 写失败时向独立 stdout 记录固定错误；两输出都失效也保持原执行失败门禁。
    pub(crate) fn emit(&self) {
        if let Err(error) = writeln!(
            io::stderr().lock(),
            "DG_SCAN_FAILURE_POSTERIOR_NON_ATOMIC {}",
            self.report
        ) {
            let _ = writeln!(
                io::stdout().lock(),
                "DG_SCAN_FAILURE_DIAGNOSTIC_OUTPUT_ERROR {}",
                Self::io_error(&error)
            );
        }
    }

    fn read_database(path: &Path, report: &mut Value, read: impl FnOnce(&Connection, &mut Value)) {
        report["read_attempted"] = json!(false);
        let connection = match Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(connection) => connection,
            Err(error) => {
                report["open"] = Self::sql_error(&error);
                return;
            }
        };
        report["open"] = json!({"status":"ok","read_only":true});
        if let Err(error) = connection.busy_timeout(Duration::ZERO) {
            report["configuration"] = Self::sql_error(&error);
        } else {
            let started = Instant::now();
            let mut checkpoints = 0;
            let configured = connection.progress_handler(
                100,
                Some(move || {
                    checkpoints += 1;
                    checkpoints >= 500 || started.elapsed() >= Duration::from_millis(100)
                }),
            );
            match configured {
                Ok(()) => {
                    report["configuration"] = json!({"status":"ok"});
                    report["read_attempted"] = json!(true);
                    read(&connection, report);
                }
                Err(error) => report["configuration"] = Self::sql_error(&error),
            }
        }
        report["clear_progress"] = match connection.progress_handler(0, None::<fn() -> bool>) {
            Ok(()) => json!({"status":"ok"}),
            Err(error) => Self::sql_error(&error),
        };
        report["close"] = match connection.close() {
            Ok(()) => json!({"status":"ok"}),
            Err((_connection, error)) => Self::sql_error(&error),
        };
    }

    fn query(
        connection: &Connection,
        sql: &str,
        parameters: impl Params,
        read: impl FnOnce(&Row<'_>) -> rusqlite::Result<Value>,
    ) -> Value {
        let started = Instant::now();
        let mut value = match connection.query_row(sql, parameters, read) {
            Ok(value) => json!({"status":"ok","value":value}),
            Err(rusqlite::Error::QueryReturnedNoRows) => json!({"status":"missing"}),
            Err(error) => Self::sql_error(&error),
        };
        value["elapsed_us"] = json!(started.elapsed().as_micros());
        value
    }

    fn sql_error(error: &rusqlite::Error) -> Value {
        let variant = match error {
            rusqlite::Error::SqliteFailure(..) => "sqlite_failure",
            rusqlite::Error::InvalidColumnType(..) => "invalid_column_type",
            rusqlite::Error::IntegralValueOutOfRange(..) => "integral_value_out_of_range",
            rusqlite::Error::FromSqlConversionFailure(..) => "from_sql_conversion_failure",
            _ => "other_rusqlite_error",
        };
        json!({"status":"error","category":"sqlite","variant":variant,"extended_code":error.sqlite_error().map(|error|error.extended_code),"primary_code":error.sqlite_error().map(|error|error.extended_code & 255)})
    }

    fn io_error(error: &io::Error) -> Value {
        json!({"status":"error","category":"io","kind":format!("{:?}",error.kind()),"raw_os_error":error.raw_os_error()})
    }
}

#[cfg(test)]
mod tests {
    use super::ScanFailureDiagnostic;
    use diskgraph_core::{BusinessError, PrincipalId};
    use diskgraph_engine::{Engine, EngineConfig, EngineError};
    use diskgraph_store::{ControlStore, SqliteSnapshotStore};
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn missing_databases_remain_missing_and_preserve_the_original_category() {
        let directory = tempfile::tempdir().unwrap();
        let original = EngineError::Business(BusinessError::Conflict);
        let diagnostic = ScanFailureDiagnostic::capture(
            directory.path(),
            "missing-job",
            "missing-scope",
            "benchmark",
            "benchmark",
            &original,
            Duration::from_secs(5),
        );
        assert_eq!(diagnostic.report["original_error"]["code"], "conflict");
        assert_eq!(diagnostic.report["control"]["open"]["primary_code"], 14);
        assert_eq!(diagnostic.report["graph"]["open"]["primary_code"], 14);
        assert!(matches!(
            original,
            EngineError::Business(BusinessError::Conflict)
        ));
        assert!(!directory.path().join("diskgraph-control.sqlite").exists());
        assert!(!directory.path().join("diskgraph.sqlite").exists());
    }

    #[test]
    fn actual_public_queued_job_is_read_without_metadata_or_durable_changes() {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data");
        let root = directory.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("benchmark").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let job = engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let before = std::fs::read(data.join("diskgraph-control.sqlite")).unwrap();
        // 此处只验证后验读取器；模拟传入分类不冒充已经执行或认领的扫描失败。
        let original = EngineError::Business(BusinessError::Conflict);
        let diagnostic = ScanFailureDiagnostic::capture(
            &data,
            &job.job_id,
            scope.as_str(),
            principal.as_str(),
            "benchmark",
            &original,
            Duration::ZERO,
        );
        assert_eq!(diagnostic.report["control"]["job"]["status"], "ok");
        assert_eq!(
            diagnostic.report["control"]["job"]["value"]["state"],
            "queued"
        );
        assert_eq!(
            diagnostic.report["control"]["job"]["value"]["fencing_token"],
            0
        );
        assert_eq!(
            diagnostic.report["control"]["scope"]["value"]["revoked"],
            false
        );
        assert_eq!(
            diagnostic.report["control"]["index_grant"]["value"]["present_for_live_policy"],
            true
        );
        assert_eq!(
            diagnostic.report["graph"]["scope_latest_pointer"]["value"]["exists"],
            false
        );
        for name in ["nodes", "search", "unix_observations"] {
            assert_eq!(
                diagnostic.report["graph"][name]["value"],
                json!({"rows":0,"is_lower_bound":false})
            );
        }
        assert_eq!(
            std::fs::read(data.join("diskgraph-control.sqlite")).unwrap(),
            before
        );
    }

    #[test]
    fn actual_exclusive_writer_is_a_separate_posterior_sqlite_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("diskgraph-control.sqlite");
        drop(ControlStore::open(&path).unwrap());
        drop(SqliteSnapshotStore::open(&directory.path().join("diskgraph.sqlite")).unwrap());
        let writer = rusqlite::Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let original = EngineError::Business(BusinessError::Conflict);
        let diagnostic = ScanFailureDiagnostic::capture(
            directory.path(),
            "missing-job",
            "missing-scope",
            "benchmark",
            "benchmark",
            &original,
            Duration::from_secs(5),
        );
        writer.execute_batch("ROLLBACK").unwrap();
        assert_eq!(diagnostic.report["original_error"]["code"], "conflict");
        assert_eq!(diagnostic.report["control"]["job"]["primary_code"], 5);
        assert_eq!(diagnostic.report["control"]["scope"]["primary_code"], 5);
        assert_eq!(diagnostic.report["control"]["close"]["status"], "ok");
        assert_eq!(
            diagnostic.report["graph"]["scope_latest_pointer"]["status"],
            "ok"
        );
        assert!(matches!(
            original,
            EngineError::Business(BusinessError::Conflict)
        ));
    }
}
