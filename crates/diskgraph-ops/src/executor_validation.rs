//! executor_validation：既有文件操作职责的原生 Rust 实现。
use crate::executor::Executor;
use crate::live_item::LiveItem;
use crate::ops_authorization::require_action;
use crate::ops_authorization::require_destination;
use crate::ops_error::OpsError;
use crate::ops_time::now_ms;
use crate::path_codec::unhex_key;
use crate::source_evidence::capture_source;
use crate::source_evidence::identity_of;
use diskgraph_core::FileActionKind;
use diskgraph_store::Plan;

impl Executor {
    /// 复核计划对象及已批准元数据。
    /// 参数：plan 为持久化计划。
    /// 返回：实时对象列表或前置条件错误。
    /// Re-resolves every planned object against the live filesystem.
    pub(super) fn resolve_live_items(&self, plan: &Plan) -> Result<Vec<LiveItem>, OpsError> {
        let mut live = Vec::new();
        let mut total = 0_u64;
        for item in &plan.items {
            let path = unhex_key(&item.locator_key)
                .ok_or_else(|| OpsError::Stale(format!("bad locator for node {}", item.node_id)))?;
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| OpsError::Stale(format!("node {}: {error}", item.node_id)))?;
            if item.identity != identity_of(&path, &metadata) {
                return Err(OpsError::Stale(format!(
                    "node {} was replaced since the plan was made",
                    item.node_id
                )));
            }
            let evidence = capture_source(&path, plan.max_bytes.saturating_sub(total))?;
            if item.identity != evidence.identity {
                return Err(OpsError::Stale(format!(
                    "node {} was replaced since the plan was made",
                    item.node_id
                )));
            }
            if item.source_fingerprint.as_deref() != Some(&evidence.fingerprint) {
                return Err(OpsError::Stale(format!(
                    "node {} changed since approval; create a fresh plan",
                    item.node_id
                )));
            }
            total = total
                .checked_add(evidence.bytes)
                .ok_or_else(|| OpsError::Stale("plan byte count overflowed".into()))?;
            if total > plan.max_bytes {
                return Err(OpsError::Stale(
                    "plan exceeded its approved byte budget".into(),
                ));
            }
            live.push(LiveItem {
                path,
                identity: evidence.identity,
                recovery_ref: item.recovery_ref.clone(),
                bytes: evidence.bytes,
                approved_version: Some(evidence.metadata),
            });
        }
        if total != plan.expected_bytes {
            return Err(OpsError::Stale(
                "source bytes no longer match the approved plan".into(),
            ));
        }
        Ok(live)
    }

    /// 执行前复核计划所需实时授权。
    /// 参数：plan 为已登记计划。
    /// 返回：允许为 Ok，否则返回拒绝。
    pub(super) fn check_plan_authorization(&self, plan: &Plan) -> Result<(), OpsError> {
        if now_ms() >= plan.expires_at_unix_ms {
            return Err(OpsError::Stale("plan expired; review a new plan".into()));
        }
        let policy = self.engine.policy_authorizer()?;
        if plan.policy_version != policy.current_version() {
            return Err(OpsError::NotAuthorized(
                "policy changed since the plan was approved".into(),
            ));
        }
        require_action(&self.engine, &plan.scope_id, &plan.principal, plan.action)?;
        match plan.action {
            FileActionKind::Move | FileActionKind::Copy => {
                let directory = plan
                    .target_locator_key
                    .as_deref()
                    .and_then(unhex_key)
                    .ok_or_else(|| OpsError::Stale("plan target is malformed".into()))?;
                require_destination(&self.engine, &directory, &plan.principal, plan.action)?;
            }
            FileActionKind::Restore => {
                let directory = match plan.target_locator_key.as_deref() {
                    Some(key) => unhex_key(key)
                        .ok_or_else(|| OpsError::Stale("restore target is malformed".into()))?,
                    None => {
                        let recovery = plan
                            .items
                            .first()
                            .and_then(|item| item.recovery_ref.as_deref())
                            .ok_or_else(|| OpsError::Stale("restore recovery is missing".into()))?;
                        let entry = self.engine.control_store()?.recovery(recovery)?;
                        let original = unhex_key(&entry.original_locator).ok_or_else(|| {
                            OpsError::Stale("restore original locator is malformed".into())
                        })?;
                        original
                            .parent()
                            .ok_or_else(|| {
                                OpsError::Stale("restore original parent is missing".into())
                            })?
                            .to_path_buf()
                    }
                };
                require_destination(&self.engine, &directory, &plan.principal, plan.action)?;
            }
            FileActionKind::Trash | FileActionKind::Purge => {}
        }
        Ok(())
    }
}
