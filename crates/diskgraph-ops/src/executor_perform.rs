//! executor_perform：既有文件操作职责的原生 Rust 实现。
use crate::atomic_publish;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::bound_path;
use crate::cross_volume_copy::CrossVolumeCopy;
use crate::executor::Executor;
use crate::fault_point::FaultPoint;
use crate::live_item::LiveItem;
use crate::operation_description::describe;
use crate::ops_authorization::require_destination;
use crate::ops_error::OpsError;
use crate::ops_time::now_ms;
use crate::path_codec::locator_key;
use crate::path_codec::unhex_key;
use crate::path_revalidation::revalidate_below;
use crate::plan_digest::plan_digest;
use crate::side::Side;
use crate::source_evidence::capture_source;
use crate::source_evidence::identity_of;
use crate::step_result::StepResult;
use crate::volume_identity::are_same_volume;
use diskgraph_core::FileActionKind;
use diskgraph_store::Plan;

impl Executor {
    /// 执行已登记意图的一项文件动作。
    /// 参数：plan 为已批准计划；item 为固定源证据；fault 为演练位置；approval_ref、operation_id 用于实时复核。
    /// 返回：单项 StepResult 或操作错误；显式清理顺序保持。
    /// Performs the single filesystem step for one item. `fault` is a drill
    /// seam; production passes `None`.
    pub(super) fn perform(
        &self,
        plan: &Plan,
        item: &LiveItem,
        fault: Option<FaultPoint>,
        approval_ref: &str,
        operation_id: &str,
    ) -> Result<StepResult, OpsError> {
        let _hydration = diskgraph_engine::content::HydrationGuard::enter()?;
        let digest = plan_digest(plan);
        // 实时检查闭包每次单独取得并释放控制库 guard；文件复制期间不外持该锁。
        let check_live = || {
            self.check_plan_authorization(plan)?;
            let control = self.engine.control_store()?;
            control
                .verify_approval(
                    approval_ref,
                    &plan.plan_id,
                    &digest,
                    &plan.principal,
                    plan.action,
                )
                .map_err(|error| OpsError::NotAuthorized(error.to_string()))?;
            if control.operation(operation_id)?.state.is_terminal() {
                return Err(OpsError::NotAuthorized(
                    "operation stopped before completion".into(),
                ));
            }
            Ok(())
        };
        check_live()?;
        let expected = plan
            .items
            .iter()
            .find(|candidate| unhex_key(&candidate.locator_key).as_deref() == Some(&item.path))
            .ok_or_else(|| OpsError::Stale("live source is not in the approved plan".into()))?;
        let verified = capture_source(&item.path, plan.max_bytes)?;
        if expected.source_fingerprint.as_deref() != Some(verified.fingerprint.as_str()) {
            return Err(OpsError::Stale("source changed before execution".into()));
        }
        check_live()?;
        match plan.action {
            FileActionKind::Move => {
                let target = self.target_for(plan, &item.path)?;
                // Component-wise revalidation: a link planted in either path
                // after planning must stop the move, not redirect it (OP-04).
                revalidate_below(&self.scope_root(plan)?, &item.path, Side::Source)
                    .map_err(|fault| OpsError::Stale(format!("source: {fault}")))?;
                let target_root = require_destination(
                    &self.engine,
                    target
                        .parent()
                        .ok_or_else(|| OpsError::Stale("target has no parent".into()))?,
                    &plan.principal,
                    plan.action,
                )?;
                revalidate_below(&target_root, &target, Side::Target)
                    .map_err(|fault| OpsError::Stale(format!("target: {fault}")))?;
                // Never overwrite: a target that appeared since planning is
                // a conflict, not something to clobber (OP-05).
                if std::fs::symlink_metadata(&target).is_ok() {
                    return Err(OpsError::TargetExists);
                }
                if are_same_volume(&item.path, &target)? {
                    check_live()?;
                    atomic_publish::rename_approved_no_replace(
                        &item.path,
                        &target,
                        &verified.metadata,
                    )?;
                    return Ok(StepResult {
                        kind: diskgraph_store::OperationItemResult::Moved,
                        detail: describe(&target, item.identity.as_deref()),
                        bytes: item.bytes,
                        recovery_ref: None,
                    });
                }
                // Cross volume (OP-05, OP-09): stage, verify, publish, and only
                // then remove the source. No cross-volume atomicity is claimed;
                // the operation record names exactly which step finished.
                self.cross_volume_move_checked(item, &target, fault, &check_live)
            }
            FileActionKind::Copy => {
                let target = self.target_for(plan, &item.path)?;
                revalidate_below(&self.scope_root(plan)?, &item.path, Side::Source)
                    .map_err(|fault| OpsError::Stale(format!("source: {fault}")))?;
                let target_root = require_destination(
                    &self.engine,
                    target
                        .parent()
                        .ok_or_else(|| OpsError::Stale("target has no parent".into()))?,
                    &plan.principal,
                    plan.action,
                )?;
                revalidate_below(&target_root, &target, Side::Target)
                    .map_err(|fault| OpsError::Stale(format!("target: {fault}")))?;
                if std::fs::symlink_metadata(&target).is_ok() {
                    return Err(OpsError::TargetExists);
                }
                // 同卷复制也使用独占 staging 和摘要验证，避免半成品及目标竞态。
                let transfer = CrossVolumeCopy::open(&target, "copy")?;
                let transferred = transfer
                    .stage_and_verify_bounded(
                        &item.path,
                        &item.identity,
                        item.bytes.min(plan.max_bytes),
                        Some(&verified.metadata),
                        &check_live,
                    )
                    .and_then(|copied| {
                        check_live()?;
                        transfer.publish().map(|()| copied)
                    });
                if let Err(error) = transferred {
                    transfer.discard();
                    return Err(error);
                }
                Ok(StepResult {
                    kind: diskgraph_store::OperationItemResult::Copied,
                    detail: describe(&target, item.identity.as_deref()),
                    bytes: item.bytes,
                    recovery_ref: None,
                })
            }
            FileActionKind::Trash => {
                // Quarantine, never delete: the object is moved into a
                // same-volume holding area and a recovery record is written.
                // If quarantine cannot be used the item fails; no path here
                // removes a file permanently (OP-06).
                let quarantine = self.quarantine_root()?;
                let recovery_ref = format!("rec-{}", uuid::Uuid::new_v4());
                let name = item
                    .path
                    .file_name()
                    .ok_or_else(|| OpsError::Stale("object has no name".into()))?;
                let held = quarantine.join(&recovery_ref).join(name);
                std::fs::create_dir_all(held.parent().unwrap())?;
                if !are_same_volume(&item.path, &held)? {
                    // Quarantine is same-volume by design: a cross-volume
                    // holding area would not preserve recoverability (OP-06).
                    return Err(OpsError::CrossVolume);
                }
                check_live()?;
                atomic_publish::rename_approved_no_replace(&item.path, &held, &verified.metadata)?;
                let entry = diskgraph_store::RecoveryEntry {
                    recovery_ref: recovery_ref.clone(),
                    operation_id: operation_id.to_owned(),
                    scope_id: plan.scope_id.clone(),
                    original_locator: locator_key(&item.path),
                    quarantine_locator: locator_key(&held),
                    identity: item.identity.clone().unwrap_or_default(),
                    created_at_unix_ms: now_ms(),
                    state: diskgraph_store::RecoveryState::Available,
                };
                self.engine.control_store()?.insert_recovery(&entry)?;
                Ok(StepResult {
                    kind: diskgraph_store::OperationItemResult::Quarantined,
                    detail: describe(&held, item.identity.as_deref()),
                    bytes: item.bytes,
                    recovery_ref: Some(recovery_ref),
                })
            }
            FileActionKind::Restore => {
                // A restore is a new plan derived from a recovery record; it
                // never reuses the original operation.
                let recovery_ref = item
                    .recovery_ref
                    .as_deref()
                    .ok_or_else(|| OpsError::Stale("restore item has no recovery".into()))?;
                let (entry, destination) = {
                    let control = self.engine.control_store()?;
                    let entry = control.recovery(recovery_ref)?;
                    if entry.state != diskgraph_store::RecoveryState::Available {
                        return Err(OpsError::Stale(format!(
                            "recovery {recovery_ref} is {:?}",
                            entry.state
                        )));
                    }
                    let held = unhex_key(&entry.quarantine_locator)
                        .ok_or_else(|| OpsError::Stale("recovery locator is malformed".into()))?;
                    // The original location wins unless the plan names
                    // another authorized destination.
                    let destination = match &plan.target_locator_key {
                        Some(key) => unhex_key(key)
                            .ok_or_else(|| OpsError::Stale("restore target is malformed".into()))?
                            .join(held.file_name().ok_or_else(|| {
                                OpsError::Stale("held object has no name".into())
                            })?),
                        None => unhex_key(&entry.original_locator).ok_or_else(|| {
                            OpsError::Stale("recovery locator is malformed".into())
                        })?,
                    };
                    (entry, destination)
                };
                let held = unhex_key(&entry.quarantine_locator)
                    .ok_or_else(|| OpsError::Stale("recovery locator is malformed".into()))?;
                let target_root = require_destination(
                    &self.engine,
                    destination
                        .parent()
                        .ok_or_else(|| OpsError::Stale("restore target has no parent".into()))?,
                    &plan.principal,
                    plan.action,
                )?;
                revalidate_below(&target_root, &destination, Side::Target)
                    .map_err(|fault| OpsError::Stale(format!("restore target: {fault}")))?;
                let metadata = std::fs::symlink_metadata(&held)
                    .map_err(|error| OpsError::Stale(format!("held object is gone: {error}")))?;
                if !entry.identity.is_empty()
                    && let Some(actual) = identity_of(&held, &metadata)
                    && entry.identity != actual
                {
                    return Err(OpsError::Stale("the held object was replaced".into()));
                }
                // A restore never overwrites what is already there (OP-06).
                if std::fs::symlink_metadata(&destination).is_ok() {
                    return Err(OpsError::TargetExists);
                }
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                check_live()?;
                atomic_publish::rename_approved_no_replace(
                    &held,
                    &destination,
                    &verified.metadata,
                )?;
                self.engine
                    .control_store()?
                    .mark_recovery_restored(recovery_ref)?;
                Ok(StepResult {
                    kind: diskgraph_store::OperationItemResult::Restored,
                    detail: describe(&destination, Some(&entry.identity)),
                    bytes: metadata.len(),
                    recovery_ref: None,
                })
            }
            FileActionKind::Purge => {
                // The identity already matched in resolve_live_items; the
                // component-wise check here closes the remaining race where a
                // planned path was relinked or re-rooted before execution
                // (OP-04). Purge is irreversible: there is no recovery record,
                // and the result says so plainly (OP-07).
                revalidate_below(&self.scope_root(plan)?, &item.path, Side::Source)
                    .map_err(|fault| OpsError::Stale(format!("purge: {fault}")))?;
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                {
                    let source = bound_path::BoundPath::open(&item.path)?;
                    let pinned = source.read()?;
                    let metadata = pinned.metadata()?;
                    if !metadata.is_file()
                        || item.identity.is_none()
                        || identity_of(&item.path, &metadata) != item.identity
                    {
                        return Err(OpsError::Stale(
                            "purge needs the approved regular file identity".into(),
                        ));
                    }
                    check_live()?;
                    source.remove_verified(&verified.metadata)?;
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                return Err(OpsError::Stale(
                    "unsupported: verified purge handles".into(),
                ));
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                Ok(StepResult {
                    kind: diskgraph_store::OperationItemResult::Purged,
                    detail: describe(&item.path, item.identity.as_deref()),
                    bytes: item.bytes,
                    recovery_ref: None,
                })
            }
        }
    }
}
