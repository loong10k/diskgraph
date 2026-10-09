//! 原生根目录最新快照的预算与编码后授权；保留旧 snapshot_id 形态。
use crate::{local_principal, native_reply, open_engine};
use diskgraph_core::{BusinessError, Locator, QueryBudget, query_deadline};
use diskgraph_engine::Engine;
use serde_json::json;
use std::path::Path;
use std::time::Instant;

#[cfg(test)]
#[path = "native_latest_tests.rs"]
mod tests;

/// 在路径解析和打开引擎前建立原请求期限。
/// 参数：数据库和根路径；返回：兼容成功文本或预算/授权失败 envelope。
pub(crate) fn legacy(database: &str, root: &str) -> String {
    native_reply::respond((|| {
        let budget = QueryBudget::default();
        let deadline = query_deadline(budget).map_err(|e| e.to_string())?;
        if root.len() > budget.max_response_bytes {
            return Err("budget_exceeded: root bytes".into());
        }
        let root = Path::new(root).canonicalize().map_err(|e| e.to_string())?;
        let engine = open_engine(database)?;
        query(&engine, &root, deadline, || {})
    })())
}

/// 查询必要标识，完整编码后由 Engine 复验范围和 revision 的实时授权。
/// 参数：引擎、规范根、原期限和编码后同步点；返回：原成功文本或错误。
pub(crate) fn query(
    engine: &Engine,
    root: &Path,
    deadline: Instant,
    before_reply: impl FnOnce(),
) -> Result<String, String> {
    let budget = QueryBudget::default();
    if root
        .as_os_str()
        .as_encoded_bytes()
        .len()
        .checked_mul(4)
        .is_none_or(|n| n > budget.max_response_bytes)
    {
        return Err("budget_exceeded: root bytes".into());
    }
    let principal = local_principal()?;
    let policy = engine
        .policy_authorizer_for_principal_until(&principal, deadline)
        .map_err(|e| e.to_string())?;
    engine
        .with_latest_snapshot_id_until(
            &Locator::from_native_path(root),
            &principal,
            &policy,
            budget,
            deadline,
            |snapshot, reads| {
                let text = native_reply::encode(&json!({"snapshot_id":snapshot}))
                    .map_err(|_| BusinessError::BudgetExceeded)?;
                if !reads.admit(0, 0, text.len()) {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                before_reply();
                Ok(text)
            },
        )
        .map_err(|e| e.to_string())
}
