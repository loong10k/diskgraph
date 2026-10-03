// 稳定 UniFFI 只读函数导出；来源：原 lib.rs 绑定入口。
use serde_json::{Value, json};
use std::path::Path;

/// Reports implemented capabilities; unsupported scopes are never claimed empty.
#[cfg_attr(
    doc,
    doc = "Reports implemented capabilities; unsupported scopes are never claimed empty.\n报告当前绑定实现的能力，未支持的平台范围明确为否。\n参数：无输入。\n返回：包含平台与只读能力标志的 JSON envelope。"
)]
#[uniffi::export]
pub fn capabilities_json() -> String {
    response(Ok(json!({
        "platform": std::env::consts::OS,
        "native_path_scan": cfg!(any(target_os = "macos", target_os = "windows", target_os = "linux")),
        "document_uri_scan": false,
        "read_only_queries": true,
        "cleanup_execution": false
    })))
}

/// Scans a caller-chosen native directory and persists an immutable snapshot.
#[cfg_attr(
    doc,
    doc = "Scans a caller-chosen native directory and persists an immutable snapshot.\n同步扫描指定本机目录并保存不可变快照。\n参数：database_path 为图库路径，root_path 为扫描根目录。\n返回：完成快照摘要或扫描失败的 JSON envelope。"
)]
#[uniffi::export]
pub fn scan_native_json(database_path: String, root_path: String) -> String {
    response(run_scan_with_cancel(
        &database_path,
        &root_path,
        &std::sync::atomic::AtomicBool::new(false),
    ))
}

/// Returns the latest snapshot ID for a native path previously scanned.
#[cfg_attr(
    doc,
    doc = "Returns the latest snapshot ID for a native path previously scanned.\n查询已授权本机根目录的最新快照。\n参数：database_path 为图库路径，root_path 为已注册根目录。\n返回：最新 snapshot_id 或空值的 JSON envelope；无权限或未绑定返回失败。"
)]
#[uniffi::export]
pub fn latest_native_snapshot_json(database_path: String, root_path: String) -> String {
    response((|| {
        let canonical = Path::new(&root_path)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let engine = open_engine(&database_path)?;
        let principal = local_principal()?;
        let policy = engine
            .policy_authorizer()
            .map_err(|error| error.to_string())?;
        let scopes = engine
            .list_scopes(&principal, &policy)
            .map_err(|error| error.to_string())?;
        let scope = scopes
            .into_iter()
            .find(|scope| {
                !scope.revoked && scope.root.to_native_path().ok().as_deref() == Some(&canonical)
            })
            .ok_or_else(|| {
                "scope is unbound or access is denied; register and reindex".to_owned()
            })?;
        let revision = engine
            .latest_revision(&scope.scope_id)
            .map_err(|error| error.to_string())?;
        let id = revision
            .map(|revision| {
                engine
                    .revision_snapshot(&revision)
                    .map(|snapshot| snapshot.id)
            })
            .transpose()
            .map_err(|error| error.to_string())?;
        Ok(json!({ "snapshot_id": id }))
    })())
}

#[cfg_attr(
    doc,
    doc = "读取父目录下按大小排序的完整有限列表。\n参数：database_path/snapshot_id 定位图库快照，parent_id 为父节点，limit 为条数上限。\n返回：旧数组形态 JSON envelope；非法上限或预算不足失败。"
)]
#[uniffi::export]
pub fn top_json(database_path: String, snapshot_id: String, parent_id: u64, limit: u32) -> String {
    if let Err(error) = bounded_limit(limit) {
        return native_reply::respond(Err(error));
    }
    native_reply::legacy(&database_path, &snapshot_id, |store, deadline| {
        native_listing::top(store, &snapshot_id, parent_id, limit, deadline)
    })
}

#[cfg_attr(
    doc,
    doc = "读取父目录的有界子节点页。\n参数：database_path/snapshot_id 定位快照，parent_id 为父节点，offset/limit 为分页范围。\n返回：含 items/next_offset 的 JSON envelope 或失败。"
)]
#[uniffi::export]
pub fn children_json(
    database_path: String,
    snapshot_id: String,
    parent_id: u64,
    offset: u64,
    limit: u32,
) -> String {
    if let Err(error) = bounded_limit(limit) {
        return native_reply::respond(Err(error));
    }
    native_reply::legacy(&database_path, &snapshot_id, |store, deadline| {
        native_listing::children(store, &snapshot_id, parent_id, offset, limit, deadline)
    })
}

#[cfg_attr(
    doc,
    doc = "读取一个已授权节点及其证据。\n参数：database_path/snapshot_id 定位快照，node_id 为目标节点。\n返回：节点与证据 JSON envelope，节点缺失为 null。"
)]
#[uniffi::export]
pub fn explain_json(database_path: String, snapshot_id: String, node_id: u64) -> String {
    native_reply::legacy(&database_path, &snapshot_id, |store, deadline| {
        let mut budget =
            diskgraph_core::QueryReadBudget::new(diskgraph_core::QueryBudget::default(), deadline)
                .map_err(|error| error.to_string())?;
        let node = store
            .node_with_budget(&snapshot_id, node_id, &mut budget)
            .map_err(|error| error.to_string())?;
        let Some(node) = node else {
            return Ok(Value::Null);
        };
        let evidence = store
            .evidence_with_budget(&snapshot_id, node_id, &mut budget)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "node": node, "evidence": evidence }))
    })
}

/// Compares a locator in two complete, compatible snapshots.
#[cfg_attr(
    doc,
    doc = "Compares a locator in two complete, compatible snapshots.\n比较两个完整兼容快照中的相同定位。\n参数：database_path 为图库，before_snapshot_id/after_snapshot_id 为双侧快照，locator_json 为定位 JSON。\n返回：双侧大小与字符串差值 JSON envelope；不兼容或超预算失败。"
)]
#[uniffi::export]
pub fn growth_json(
    database_path: String,
    before_snapshot_id: String,
    after_snapshot_id: String,
    locator_json: String,
) -> String {
    native_growth::legacy(
        &database_path,
        &before_snapshot_id,
        &after_snapshot_id,
        &locator_json,
    )
}

/// Returns candidates for review only, never paths to execute automatically.
#[cfg_attr(
    doc,
    doc = "Returns candidates for review only, never paths to execute automatically.\n读取仅供审阅的候选列表。\n参数：database_path/snapshot_id 定位快照，target_bytes 为期望候选总字节数。\n返回：完整候选数组 JSON envelope；预算截断失败，不授权文件执行。"
)]
#[uniffi::export]
pub fn candidates_json(database_path: String, snapshot_id: String, target_bytes: u64) -> String {
    native_reply::legacy_with_revision(&database_path, &snapshot_id, |store, revision, deadline| {
        let selection = store
            .candidate_selection_for_revision_until(
                revision,
                target_bytes,
                diskgraph_core::QueryBudget::default(),
                deadline,
            )
            .map_err(|error| error.to_string())?;
        if selection.truncated.is_some() {
            return Err(
                "candidate query exceeded budget; use the session API for partial results".into(),
            );
        }
        let candidates: Vec<_> = selection
            .candidates
            .into_iter()
            .map(|(node, evidence)| json!({"node":node,"evidence":evidence}))
            .collect();
        Ok(json!(candidates))
    })
}

/// Spawns a native-path scan on a worker thread and returns a handle at
/// once. The scan uses the same code path as `scan_native_json`; the only
/// difference is who waits.
#[cfg_attr(
    doc,
    doc = "Spawns a native-path scan on a worker thread and returns a handle at\nonce. The scan uses the same code path as `scan_native_json`; the only\ndifference is who waits.\n在线程中启动本机扫描，立即返回可轮询句柄。\n参数：database_path 为图库路径，root_path 为扫描根目录。\n返回：共享 JobHandle，扫描结果通过句柄读取。"
)]
#[uniffi::export]
pub fn spawn_scan_json(database_path: String, root_path: String) -> std::sync::Arc<JobHandle> {
    spawn_job(move |cancel, progress| {
        let engine = std::sync::Arc::new(open_engine(&database_path)?);
        run_scan_on_engine(engine, &root_path, cancel, progress)
    })
}
