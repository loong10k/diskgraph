//! 只读连接配置合并的隔离对照；不改变生产路径或代表全平台性能验收。
use crate::SqliteSnapshotStore;
use rusqlite::{Connection, OpenFlags};
use std::time::{Duration, Instant};

#[test]
#[ignore = "isolated release preparation experiment; not production acceptance"]
fn separate_and_batched_configuration_total_costs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let mut samples = [Vec::new(), Vec::new()];
    // 交替先后顺序；每次完整创建新连接，不复用旧归属观察或消费事务。
    for round in 0..500 {
        for variant in [round % 2, 1 - round % 2] {
            let deadline = Instant::now() + Duration::from_secs(5);
            let started = Instant::now();
            assert!(Instant::now() < deadline);
            let connection = Connection::open_with_flags(
                &path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap();
            connection
                .busy_timeout(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(1)),
                )
                .unwrap();
            if variant == 0 {
                connection
                    .pragma_update(None, "temp_store", "FILE")
                    .unwrap();
                connection.pragma_update(None, "cache_size", -8192).unwrap();
            } else {
                connection
                    .execute_batch("PRAGMA temp_store=FILE; PRAGMA cache_size=-8192;")
                    .unwrap();
            }
            connection
                .progress_handler(1000, Some(move || Instant::now() >= deadline))
                .unwrap();
            assert!(Instant::now() < deadline);
            let reader = SqliteSnapshotStore { connection };
            assert!(
                !reader
                    .revision_ownership_matches("missing", "server", "scope")
                    .unwrap()
            );
            // 两组均核验实际配置，不能用更低安全/资源约束获取较快数字。
            assert_eq!(
                reader
                    .connection
                    .pragma_query_value(None, "temp_store", |row| row.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                reader
                    .connection
                    .pragma_query_value(None, "cache_size", |row| row.get::<_, i64>(0))
                    .unwrap(),
                -8192
            );
            assert!(reader.connection.is_readonly("main").unwrap());
            drop(reader);
            samples[variant].push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    for values in &mut samples {
        values.sort_by(f64::total_cmp);
    }
    println!(
        "{}",
        serde_json::json!({
            "experiment":"separate versus batched identical PRAGMAs plus missing ownership and close",
            "os":std::env::consts::OS,"profile":if cfg!(debug_assertions) {"debug"} else {"release"},
            "samples_per_variant":500,"production_acceptance":false,
            "separate":{"p50_ms":samples[0][249],"p95_ms":samples[0][474]},
            "batched":{"p50_ms":samples[1][249],"p95_ms":samples[1][474]},
            "limits":"synthetic metadata; configuration assertions included; no concurrent load or positive selection"
        })
    );
}
