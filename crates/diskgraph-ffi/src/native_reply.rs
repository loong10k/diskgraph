//! 本机查询共用绝对期限、有限响应编码和实际 revision 的终态授权。
use crate::{local_principal, open_engine};
use diskgraph_core::{QueryBudget, measure_json_bounded};
use diskgraph_engine::Engine;
use diskgraph_store::SqliteSnapshotStore;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

/// 复用请求期限执行本机查询；实际成功文本在最后授权前已完成编码。
/// 参数：engine/snapshot 为真实资源，cancel 为关闭标志，read 接收与终态授权相同的固定 revision。
/// 返回：已编码成功文本或失败；旧数组入口到期失败，会话有诊断的结果保留前缀。
pub(crate) fn query_with_revision(
    engine: &Engine,
    snapshot: &str,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
    read: impl FnOnce(&SqliteSnapshotStore, &str, Instant) -> Result<Value, String>,
    before_reply: impl FnOnce(),
) -> Result<String, String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("session closed".into());
    }
    let principal = local_principal()?;
    engine
        .authorize_snapshot_until(
            snapshot,
            &principal,
            &engine.policy_authorizer().map_err(|e| e.to_string())?,
            deadline,
        )
        .map_err(|e| e.to_string())?;
    let reader = engine
        .revision_reader_with_cancel_until(cancel.clone(), deadline)
        .map_err(|e| e.to_string())?;
    let revision = reader
        .revision_for_snapshot(snapshot)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "permission_denied: unbound snapshot".to_owned())?;
    let result = read(&reader, &revision, deadline);
    if cancel.load(Ordering::SeqCst) {
        return Err("session closed".into());
    }
    // 即使预算/诊断无法容纳也复检权限；失败结果不向客户端返回部分数据。
    let mut answer = match result {
        Ok(answer) => answer,
        Err(error) => {
            engine
                .finalize_revision_read_until(
                    &revision,
                    &principal,
                    &engine.policy_authorizer().map_err(|e| e.to_string())?,
                    deadline,
                )
                .map_err(|e| e.to_string())?;
            return Err(error);
        }
    };
    let encoded = encode(&answer);
    // 同步点只表示成功文本已编码，前段编码失败不能冒充终态阶段。
    if encoded.is_ok() {
        before_reply();
    }
    let live = engine
        .finalize_revision_read_until(
            &revision,
            &principal,
            &engine.policy_authorizer().map_err(|e| e.to_string())?,
            deadline,
        )
        .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::SeqCst) {
        return Err("session closed".into());
    }
    let text = encoded?;
    if live {
        return Ok(text);
    }
    if !answer.get("complete").is_some_and(Value::is_boolean) {
        return Err("timeout: query deadline exceeded".into());
    }
    answer["complete"] = json!(false);
    answer["truncated"] = json!("deadline");
    let encoded = encode(&answer);
    engine
        .finalize_revision_read_until(
            &revision,
            &principal,
            &engine.policy_authorizer().map_err(|e| e.to_string())?,
            deadline,
        )
        .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::SeqCst) {
        return Err("session closed".into());
    }
    encoded
}

/// 保留两参数内部回调；参数为原请求上下文，返回共享终态校验后的文本。
pub(crate) fn query(
    engine: &Engine,
    snapshot: &str,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
    read: impl FnOnce(&SqliteSnapshotStore, Instant) -> Result<Value, String>,
    before_reply: impl FnOnce(),
) -> Result<String, String> {
    query_with_revision(
        engine,
        snapshot,
        deadline,
        cancel,
        |store, _, until| read(store, until),
        before_reply,
    )
}

/// 对成功 envelope 计量并编码；参数为数据，返回有限文本或明确预算错误。
pub(crate) fn encode(data: &Value) -> Result<String, String> {
    let cap = QueryBudget::default().max_response_bytes;
    // 先有限计量借用数据，阻止构造 envelope 时复制超大字符串。
    if measure_json_bounded(data, cap)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Err("budget_exceeded: response bytes".into());
    }
    let reply = json!({"schema_version": 1, "ok": true, "data": data});
    if measure_json_bounded(&reply, QueryBudget::default().max_response_bytes)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Err("budget_exceeded: response bytes".into());
    }
    serde_json::to_string(&reply).map_err(|error| error.to_string())
}

/// 保留原有成功/失败 schema；成功文本已在请求终态检查前编码。
/// 参数：result 为完成授权的文本或错误。返回：原生接口 JSON 文本。
pub(crate) fn respond(result: Result<String, String>) -> String {
    match result {
        Ok(text) => text,
        Err(error) => failure(&error),
    }
}

fn failure(error: &str) -> String {
    let cap = QueryBudget::default().max_response_bytes;
    // 失败详情也按转义后的真实字节计量；在构造拥有的 envelope 前检查原字符串。
    if matches!(measure_json_bounded(error, cap), Ok(Some(_))) {
        let reply = json!({"schema_version":1,"ok":false,"error":error});
        if matches!(measure_json_bounded(&reply, cap), Ok(Some(_))) {
            return reply.to_string();
        }
    }
    // 保留 schema 与失败语义；详情无法表示时只返回固定、有限的诊断。
    json!({"schema_version":1,"ok":false,"error":"query failed: error diagnostic exceeds response byte budget"}).to_string()
}

/// 旧 FFI 单次入口从打开引擎前开始计时，保持导出签名与失败 schema。
/// 参数：path/snapshot 为可信本机请求，read 使用同一期限。返回：完整成功或明确失败文本。
pub(crate) fn legacy(
    path: &str,
    snapshot: &str,
    read: impl FnOnce(&SqliteSnapshotStore, Instant) -> Result<Value, String>,
) -> String {
    respond((|| {
        let deadline = diskgraph_core::query_deadline(QueryBudget::default())
            .map_err(|error| error.to_string())?;
        let engine = open_engine(path)?;
        query(
            &engine,
            snapshot,
            deadline,
            Arc::new(AtomicBool::new(false)),
            read,
            || {},
        )
    })())
}

/// 旧导出候选入口使用首次解析的 revision；参数为路径、snapshot 与固定 revision 回调。
/// 返回：沿用原 JSON envelope，不在数据读取阶段重新选择最新批次。
pub(crate) fn legacy_with_revision(
    path: &str,
    snapshot: &str,
    read: impl FnOnce(&SqliteSnapshotStore, &str, Instant) -> Result<Value, String>,
) -> String {
    respond((|| {
        let deadline = diskgraph_core::query_deadline(QueryBudget::default())
            .map_err(|error| error.to_string())?;
        let engine = open_engine(path)?;
        query_with_revision(
            &engine,
            snapshot,
            deadline,
            Arc::new(AtomicBool::new(false)),
            read,
            || {},
        )
    })())
}
