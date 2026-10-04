//! Linux 配对成本验收辅助；来源：原生 Rust hardening_benchmark 与 FS-02 持久观察。
use diskgraph_engine::Engine;
use serde_json::{Value, json};
use std::path::Path;

/// 记录参数 data 下两库三类文件的逻辑长度；返回字节快照，不表示物理写入量。
pub(crate) fn storage(data: &Path) -> Value {
    let mut sizes = serde_json::Map::new();
    for database in ["diskgraph.sqlite", "diskgraph-control.sqlite"] {
        for suffix in ["", "-wal", "-shm"] {
            let name = format!("{database}{suffix}");
            let bytes = std::fs::metadata(data.join(&name)).map_or(0, |meta| meta.len());
            sizes.insert(name, json!(bytes));
        }
    }
    Value::Object(sizes)
}

/// 参数为扫描前、扫描后和查询后尺寸；返回原值及有符号阶段差值，允许 WAL 收缩。
pub(crate) fn phases(before: Value, scanned: Value, queried: Value) -> Value {
    let delta = |left: &Value, right: &Value| {
        left.as_object()
            .unwrap()
            .keys()
            .map(|key| {
                let difference = i128::from(right[key].as_u64().unwrap())
                    - i128::from(left[key].as_u64().unwrap());
                (key.clone(), json!(i64::try_from(difference).unwrap()))
            })
            .collect::<serde_json::Map<String, Value>>()
    };
    json!({"before_scan":before,"after_scan":scanned,"after_queries":queried,
        "scan_signed_delta":delta(&before,&scanned),
        "query_signed_delta":delta(&scanned,&queried)})
}

/// 对参数所指真实扫描检查普通文件捕获数及首尾强身份；返回资格见证，缺能力直接失败。
pub(crate) fn qualify(
    engine: &Engine,
    data: &Path,
    revision: &str,
    count: usize,
    shape: &str,
) -> Value {
    if !cfg!(target_os = "linux") {
        panic!("paired capture measurement requires native Linux");
    }
    assert!(count > 0 && matches!(shape, "wide" | "deep"));
    let snapshot = engine.revision_snapshot(revision).unwrap();
    let connection = rusqlite::Connection::open_with_flags(
        data.join("diskgraph.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (rows, captured, gaps): (i64, i64, i64) = connection.query_row(
        "SELECT count(*), coalesce(sum(observation_raw IS NOT NULL AND gap IS NULL),0), coalesce(sum(gap IS NOT NULL),0) FROM node_unix_observations WHERE snapshot_id=?1",
        [&snapshot.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(
        (rows, captured, gaps),
        (count as i64, count as i64, 0),
        "Unsupported/gap must not be measured as successful native capture"
    );
    let store = diskgraph_store::SqliteSnapshotStore::open(&data.join("diskgraph.sqlite")).unwrap();
    let mut witnesses = Vec::new();
    for index in [0, count - 1] {
        let mut path = std::path::PathBuf::new();
        if shape == "deep" {
            for _ in 0..=index {
                path.push("d");
            }
        }
        path.push(format!("file-{index:06}"));
        let node = engine.revision_node_at(revision, &path).unwrap().unwrap();
        let mut reads = diskgraph_core::QueryReadBudget::new(
            diskgraph_core::QueryBudget::default(),
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        )
        .unwrap();
        let record = store
            .unix_observation_bounded(&snapshot.id, node.id, &mut reads)
            .unwrap()
            .unwrap();
        assert!(record.gap.is_none());
        let observation = record
            .observation
            .expect("persisted Captured observation required");
        observation.validate().unwrap();
        assert_eq!(observation.length(), 32);
        assert!(matches!(
            observation.epoch(),
            diskgraph_core::IndexedFileEpoch::LinuxHandle { .. }
        ));
        witnesses.push(json!({"relative_path":path,"node_id":node.id,"epoch":observation.epoch()}));
    }
    json!({"qualified":true,"method":"linux_handle","captured_files":captured,
        "gaps":gaps,"snapshot_id":snapshot.id,"witnesses":witnesses})
}
