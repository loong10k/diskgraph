//! 旧原生 growth 的双侧窄读、共同预算与成组终态授权；来源：D25 / Q-02。

use crate::{local_principal, native_reply, open_engine};
use diskgraph_core::{DiskGraph, QueryBudget, QueryReadBudget, ResourceLocator, query_deadline};
use diskgraph_engine::Engine;
use serde_json::{Value, json};
use std::time::Instant;

/// 保持旧 growth 导出数据形态，从解析和打开引擎前建立期限。
/// 参数：path、双侧 snapshot 和 locator JSON。返回：有界成功/失败 envelope。
pub(crate) fn legacy(path: &str, before: &str, after: &str, locator: &str) -> String {
    native_reply::respond((|| {
        let deadline = query_deadline(QueryBudget::default()).map_err(|e| e.to_string())?;
        if locator.len() > QueryBudget::default().max_response_bytes {
            return Err("budget_exceeded: locator bytes".into());
        }
        let engine = open_engine(path)?;
        query(&engine, [before, after], locator, deadline, || {})
    })())
}

/// 双侧真实资源授权后读取同一 locator，完整编码后复核所有权限。
/// 参数：engine、snapshot、locator、原 deadline 和末检前同步闭包。
/// 返回：原 before/after/字符串 delta 或 null；任一撤权/到期拒绝所有数据。
pub(crate) fn query(
    engine: &Engine,
    snapshots: [&str; 2],
    locator_json: &str,
    deadline: Instant,
    before_reply: impl FnOnce(),
) -> Result<String, String> {
    let budget = QueryBudget::default();
    if locator_json.len() > budget.max_response_bytes {
        return Err("budget_exceeded: locator bytes".into());
    }
    let locator: ResourceLocator = serde_json::from_str(locator_json).map_err(|e| e.to_string())?;
    let principal = local_principal()?;
    let policy = engine.policy_authorizer().map_err(|e| e.to_string())?;
    let reader = engine
        .revision_reader_with_cancel_until(
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            deadline,
        )
        .map_err(|e| e.to_string())?;
    let mut revisions = Vec::with_capacity(2);
    for snapshot in snapshots {
        let revision = reader
            .revision_for_snapshot(snapshot)
            .map_err(|e| e.to_string())?
            .ok_or("permission_denied: unbound history requires reindexing")?;
        engine
            .authorize_revision_until(None, &revision, &principal, &policy, deadline)
            .map_err(|e| e.to_string())?;
        revisions.push(revision);
    }
    let result = (|| {
        let mut ledger = QueryReadBudget::new(budget, deadline).map_err(|e| e.to_string())?;
        let mut load = |snapshot: &str| -> Result<DiskGraph, String> {
            Ok(DiskGraph {
                snapshot: reader
                    .snapshot_with_budget(snapshot, &mut ledger)
                    .map_err(|e| e.to_string())?,
                nodes: reader
                    .node_by_locator_with_budget(snapshot, &locator, &mut ledger)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .collect(),
                evidence: Vec::new(),
            })
        };
        let before = load(snapshots[0])?;
        let after = load(snapshots[1])?;
        let data = match after.growth(&before, &locator) {
            Some(growth) => json!({"before":growth.before,"after":growth.after,
                "delta_bytes":growth.delta_bytes.to_string()}),
            None => Value::Null,
        };
        native_reply::encode(&data)
    })();
    // 仅已编码成功数据到达此同步点；错误路径仍继续执行同样的终态授权。
    if result.is_ok() {
        before_reply();
    }
    // 成组复检使用已解析的实际 revision；第二侧检查不能留下第一侧的旧授权。
    let live = engine
        .finalize_revisions_read_until(
            &[&revisions[0], &revisions[1]],
            &principal,
            &engine.policy_authorizer().map_err(|e| e.to_string())?,
            deadline,
        )
        .map_err(|e| e.to_string())?;
    if !live {
        return Err("timeout: query deadline exceeded".into());
    }
    result
}
