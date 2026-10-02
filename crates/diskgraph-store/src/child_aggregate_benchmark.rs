//! 隔离 release 夹具量化目录精确统计的读收益、迁移空间与发布 fencing 锁成本。

use crate::{ControlStore, JobKind, SqliteSnapshotStore, tests::graph};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

fn bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}
fn timed(mut action: impl FnMut()) -> serde_json::Value {
    let mut values: Vec<f64> = (0..100)
        .map(|_| {
            let started = Instant::now();
            action();
            started.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    values.sort_by(f64::total_cmp);
    serde_json::json!({"p50_ms":values[49],"p95_ms":values[94],"samples":values.len()})
}

// 一毫秒采样并非严格临时空间上界；Unix 描述符包含已 unlink 的 SQLite 排序文件。
fn sqlite_temp_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        let fd_dir = if cfg!(target_os = "macos") {
            "/dev/fd"
        } else {
            "/proc/self/fd"
        };
        let mut total = 0u64;
        for entry in std::fs::read_dir(fd_dir).ok()?.flatten() {
            let Ok(metadata) = std::fs::metadata(entry.path()) else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            #[cfg(target_os = "macos")]
            let name = {
                let fd = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<i32>().ok())?;
                let mut buffer = [0 as libc::c_char; 1024];
                // F_GETPATH 只填充 PATH_MAX 字节缓冲；成功结果是零结尾路径。
                let result = unsafe { libc::fcntl(fd, libc::F_GETPATH, buffer.as_mut_ptr()) };
                if result == 0 {
                    Some(
                        unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
                            .to_string_lossy()
                            .into_owned(),
                    )
                } else {
                    None
                }
            };
            #[cfg(not(target_os = "macos"))]
            let name = std::fs::read_link(entry.path())
                .ok()
                .map(|p| p.to_string_lossy().into_owned());
            if name.as_ref().is_none_or(|name| name.contains("etilqs")) {
                total = total.saturating_add(metadata.len());
            }
        }
        Some(total)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn observed_space<T>(path: &Path, work: impl FnOnce() -> T) -> (T, u64, Option<u64>) {
    let stop = Arc::new(AtomicBool::new(false));
    let wal = Arc::new(AtomicU64::new(0));
    let temp = Arc::new(AtomicU64::new(0));
    let supported = sqlite_temp_bytes().is_some();
    let sampler = {
        let stop = stop.clone();
        let wal = wal.clone();
        let temp = temp.clone();
        let path = path.with_extension("sqlite-wal");
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                wal.fetch_max(bytes(&path), Ordering::Relaxed);
                temp.fetch_max(sqlite_temp_bytes().unwrap_or(0), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        })
    };
    let value = work();
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    (
        value,
        wal.load(Ordering::Relaxed),
        supported.then(|| temp.load(Ordering::Relaxed)),
    )
}

#[test]
#[ignore = "isolated release 20k/200k aggregation, migration and fenced publication measurements"]
fn measure_directory_aggregate_costs() {
    let mut results = Vec::new();
    for count in [20_000u64, 200_000] {
        let directory = tempfile::tempdir().unwrap();
        let mut fixture = graph("legacy", count * (count + 1) / 2);
        let sample = fixture.nodes.pop().unwrap();
        fixture.nodes.extend((1..=count).map(|size| {
            let mut node = sample.clone();
            node.id = size + 1;
            node.name = format!("file-{size:09}");
            node.kind = diskgraph_core::NodeKind::File;
            node.subtree_bytes = size;
            node.direct_bytes = size;
            node.locator = diskgraph_core::ResourceLocator::NativePath(format!(
                "/tmp/diskgraph-test/{}",
                node.name
            ));
            node
        }));
        let path = directory.path().join("legacy.sqlite");
        let mut legacy = SqliteSnapshotStore::open(&path).unwrap();
        legacy.save(&fixture).unwrap();
        // 恢复真正 v8 结构并压缩夹具，避免已删除的 v9 表空闲页抬高基线。
        legacy.connection.execute_batch(
            "DROP TRIGGER snapshots_require_count_writer; ALTER TABLE snapshots DROP COLUMN count_schema;
             DROP TABLE child_size_prefix; DROP TABLE directory_counts; DROP TABLE snapshot_counts;
             DROP INDEX nodes_by_known_parent_size; DROP INDEX nodes_by_unknown_parent; DROP INDEX nodes_by_parent_size;
             CREATE INDEX nodes_by_parent_size ON nodes(snapshot_id,parent_id,subtree_bytes DESC,name ASC);
             VACUUM; PRAGMA wal_checkpoint(TRUNCATE);"
        ).unwrap();
        legacy.connection.execute_batch(&format!("CREATE INDEX nodes_by_unknown_parent ON nodes(snapshot_id,parent_id) WHERE NOT ({}); PRAGMA user_version=8; PRAGMA wal_checkpoint(TRUNCATE);",crate::directory_aggregates::KNOWN_SIZE)).unwrap();
        let before_counts = timed(|| {
            let actual:(i64,i64)=legacy.connection.query_row("SELECT COUNT(*),SUM(CASE WHEN subtree_bytes>=?1 THEN 1 ELSE 0 END) FROM nodes WHERE snapshot_id='legacy' AND parent_id=1",[(count/2) as i64],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
            assert_eq!(actual, (count as i64, (count / 2 + 1) as i64));
        });
        let before_db = bytes(&path);
        drop(legacy);
        let started = Instant::now();
        let ((upgraded, backup), migration_wal, migration_temp) = observed_space(&path, || {
            SqliteSnapshotStore::open_with_backup(&path, &directory.path().join("backups")).unwrap()
        });
        let migration_ms = started.elapsed().as_secs_f64() * 1000.0;
        let after_counts = timed(|| {
            assert_eq!(
                upgraded.child_counts("legacy", 1, count / 2).unwrap(),
                (count, count / 2 + 1)
            )
        });
        upgraded
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        let after_db = bytes(&path);

        let publish_path = directory.path().join("publish.sqlite");
        let mut publisher = SqliteSnapshotStore::open(&publish_path).unwrap();
        fixture.snapshot.id = "published".into();
        publisher
            .append_staging_nodes("staging", &fixture.nodes)
            .unwrap();
        publisher
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        let mut control = ControlStore::open(&directory.path().join("control.sqlite")).unwrap();
        let scope = control
            .register_scope(
                &diskgraph_core::Locator::from_native_path(Path::new("/tmp/diskgraph-test")),
                None,
            )
            .unwrap();
        let job = control
            .create_job(
                &scope,
                JobKind::Index,
                &diskgraph_core::PrincipalId::new("benchmark").unwrap(),
            )
            .unwrap();
        let job = control.claim_job_once(&job.job_id, "owner").unwrap();
        let staged_db = bytes(&publish_path);
        let started = Instant::now();
        let (_, publish_wal, publish_temp) = observed_space(&publish_path, || {
            control
                .with_job_fence(&job.job_id, "owner", job.fencing_token, || {
                    publisher.publish_revision_owned(
                        "staging",
                        &fixture,
                        "revision",
                        1,
                        Some(("server", scope.as_str())),
                    )
                })
                .unwrap()
        });
        let fenced_publish_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(publisher.node_count("published").unwrap(), count + 1);
        let result = serde_json::json!({"profile":"release","os":std::env::consts::OS,"arch":std::env::consts::ARCH,"fixture":"synthetic immutable metadata","publish_phase_includes_sampler_validation_commit_checkpoint":true,"children":count,"distinct_sizes":count,"before_counts":before_counts,"after_counts":after_counts,"migration_including_backup_ms":migration_ms,"db_before_migration_bytes":before_db,"db_after_migration_bytes":after_db,"backup_bytes":bytes(&backup.unwrap()),"migration_sampled_peak_wal_bytes":migration_wal,"migration_sampled_peak_sqlite_temp_bytes":migration_temp,"owned_staging_fenced_publish_ms":fenced_publish_ms,"staging_db_before_publish_bytes":staged_db,"db_after_publish_bytes":bytes(&publish_path),"publish_sampled_peak_wal_bytes":publish_wal,"publish_sampled_peak_sqlite_temp_bytes":publish_temp,"space_sampling_interval_ms":1,"temp_sampling_is_upper_bound":false});
        println!("{result}");
        results.push(result);
    }
    if let Some(output) = std::env::var_os("DISKGRAPH_AGGREGATE_BENCHMARK_OUTPUT") {
        std::fs::write(output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
}
