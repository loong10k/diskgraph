//! plan_builder：既有文件操作职责的原生 Rust 实现。
use crate::ops_authorization::require_action;
use crate::ops_authorization::require_destination;
use crate::ops_error::OpsError;
use crate::ops_time::now_ms;
use crate::path_codec::canonical_dir;
use crate::path_codec::locator_key;
use crate::path_codec::unhex_key;
use crate::plan_digest::plan_digest;
use crate::plan_request::PlanRequest;
use crate::raw_path::RawPath;
use crate::resolved::{Resolved, drop_nested};
use crate::source_evidence::SourceEvidence;
use crate::source_evidence::capture_source;
use crate::source_evidence::identity_of;
use diskgraph_core::Authorizer;
use diskgraph_core::BusinessError;
use diskgraph_core::Decision;
use diskgraph_core::FileActionKind;
use diskgraph_core::Permission;
use diskgraph_core::PrincipalId;
use diskgraph_core::ScopeId;
use diskgraph_engine::{Engine, EngineError};
use diskgraph_store::Plan;
use diskgraph_store::PlanItem;
use diskgraph_store::RecoveryRule;
use diskgraph_store::SqliteSnapshotStore;
use diskgraph_store::StoreError;
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 从已发布修订读取源证据并持久化不可变计划，不在构建阶段修改源文件。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::PlanBuilder`，保留既有语义。
/// Builds immutable plans from a request against a published revision.
pub struct PlanBuilder {
    engine: std::sync::Arc<Engine>,
    /// 仅测试强制最终授权锁等待；生产构建没有此字段或回调。
    #[cfg(test)]
    pub(super) metadata_read_observer: Option<Box<dyn Fn() + Send + Sync>>,
}

impl PlanBuilder {
    /// 创建原有状态对象。
    /// 参数：engine 为计划查询和控制库写入使用的共享引擎。
    /// 返回：持有相同依赖的新对象。
    pub fn new(engine: std::sync::Arc<Engine>) -> Self {
        Self {
            engine,
            #[cfg(test)]
            metadata_read_observer: None,
        }
    }

    /// 构建可恢复隔离计划。
    /// 参数：scope_id、principal、node_ids 和 max_bytes 指定范围、主体、对象与上限。
    /// 返回：记录源证据及恢复规则的持久计划或错误。
    /// Resolves a request into a plan and persists it. The request names node
    /// ids inside the scope's current revision; the builder resolves them to
    /// live paths and identities, drops parent/child overlaps, and writes a
    /// digest over the exact result. No file is opened for writing here.
    pub fn build_trash_plan(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        node_ids: &[u64],
        max_bytes: u64,
    ) -> Result<Plan, OpsError> {
        let plan = self.build(PlanRequest {
            scope_id,
            principal,
            action: FileActionKind::Trash,
            node_ids,
            target: None,
            max_bytes,
            recovery: RecoveryRule::Quarantine,
        })?;
        Ok(plan)
    }

    /// 构建目标目录移动计划。
    /// 参数：scope_id、principal、node_ids 指定源范围和对象；target 为目标目录；max_bytes 为读取上限；action 必须是 Move 或 Copy。
    /// 返回：经权限及源证据复核的持久计划或错误。
    /// Resolves a move/copy plan into `target`.
    pub fn build_move_plan(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        node_ids: &[u64],
        target: &Path,
        max_bytes: u64,
        action: FileActionKind,
    ) -> Result<Plan, OpsError> {
        if !matches!(action, FileActionKind::Move | FileActionKind::Copy) {
            return Err(OpsError::ActionMismatch);
        }
        if target
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(OpsError::NotAuthorized(
                "destination contains a parent component".into(),
            ));
        }
        let recovery = if action == FileActionKind::Move {
            RecoveryRule::MoveBack
        } else {
            RecoveryRule::NoneNeeded
        };
        // Resolve the destination now, so the plan records the same canonical
        // form as the objects it names. A caller-supplied path through a system
        // alias would otherwise not share a prefix with the scope root.
        let resolved = canonical_dir(target);
        let plan = self.build(PlanRequest {
            scope_id,
            principal,
            action,
            node_ids,
            target: Some(&resolved),
            max_bytes,
            recovery,
        })?;
        Ok(plan)
    }

    /// 构建不可恢复永久删除计划。
    /// 参数：scope_id、principal、node_ids 和 max_bytes 指定对象与预算。
    /// 返回：带明确不可恢复规则的持久计划或错误。
    /// Resolves a purge plan over exact objects. A purge is planned like any
    /// other action, but applying it requires an approval issued by the
    /// configured purge authority, and no recovery record is written (OP-07).
    pub fn build_purge_plan(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        node_ids: &[u64],
        max_bytes: u64,
    ) -> Result<Plan, OpsError> {
        self.build(PlanRequest {
            scope_id,
            principal,
            action: FileActionKind::Purge,
            node_ids,
            target: None,
            max_bytes,
            recovery: RecoveryRule::NoneNeeded,
        })
    }

    /// 根据恢复引用构建还原计划。
    /// 参数：scope_id、principal 指定范围和主体；recovery_ref 指定单个恢复条目；target 可覆盖原有还原目录。
    /// 返回：经来源及目标复核的持久计划或错误。
    /// Derives a fresh plan that puts a quarantined object back. The original
    /// location is the default; `target` names another authorized destination.
    /// Planning never modifies the recovery record itself.
    pub fn build_restore_plan(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        recovery_ref: &str,
        target: Option<&Path>,
    ) -> Result<Plan, OpsError> {
        if target.is_some_and(|path| {
            path.components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        }) {
            return Err(OpsError::NotAuthorized(
                "restore target contains a parent component".into(),
            ));
        }
        if self.engine.scope(scope_id)?.revoked
            || !matches!(
                self.engine.policy_authorizer()?.decide(
                    principal,
                    &Permission::MetadataRead,
                    scope_id
                ),
                Decision::Allowed
            )
        {
            return Err(OpsError::NotAuthorized(
                "recovery scope metadata grant is required".into(),
            ));
        }
        require_action(&self.engine, scope_id, principal, FileActionKind::Restore)?;
        let entry = {
            let control = self.engine.control_store()?;
            // An object with no recovery record — above all a purged one — is
            // gone for good; the caller hears that, never a fabricated plan
            // that pretends a restore is possible (OP-07).
            match control.recovery(recovery_ref) {
                Ok(entry) => entry,
                Err(StoreError::RecoveryNotFound(_)) => {
                    return Err(OpsError::Irrecoverable(
                        "no recovery record exists; a purged object cannot be restored".into(),
                    ));
                }
                Err(error) => return Err(error.into()),
            }
        };
        if entry.scope_id != *scope_id {
            return Err(OpsError::NotAuthorized(
                "recovery belongs to another scope".into(),
            ));
        }
        if entry.state != diskgraph_store::RecoveryState::Available {
            return Err(OpsError::Stale(format!(
                "recovery {recovery_ref} is {:?}",
                entry.state
            )));
        }
        let held = unhex_key(&entry.quarantine_locator)
            .ok_or_else(|| OpsError::Stale("recovery locator is malformed".into()))?;
        let original = unhex_key(&entry.original_locator)
            .ok_or_else(|| OpsError::Stale("original locator is malformed".into()))?;
        let destination_dir = target.unwrap_or_else(|| original.parent().unwrap_or(&original));
        require_destination(
            &self.engine,
            destination_dir,
            principal,
            FileActionKind::Restore,
        )?;
        let metadata = std::fs::symlink_metadata(&held)
            .map_err(|error| OpsError::Stale(format!("held object is gone: {error}")))?;
        let item = PlanItem {
            node_id: 0,
            locator_key: locator_key(&held),
            identity: Some(entry.identity.clone()),
            source_fingerprint: Some(capture_source(&held, metadata.len())?.fingerprint),
            includes_descendants: false,
            recovery_ref: Some(recovery_ref.to_owned()),
        };
        // Resolve the policy epoch before taking the control lock again.
        let policy_version = self.engine.policy_authorizer()?.current_version();
        let plan = Plan {
            plan_id: format!("plan-{}", uuid::Uuid::new_v4()),
            scope_id: scope_id.clone(),
            principal: principal.clone(),
            action: FileActionKind::Restore,
            items: vec![item],
            target_locator_key: Some(locator_key(&canonical_dir(destination_dir))),
            policy_version,
            max_bytes: metadata.len().max(1),
            created_at_unix_ms: now_ms(),
            expires_at_unix_ms: now_ms() + 15 * 60_000,
            recovery: RecoveryRule::NoneNeeded,
            expected_bytes: metadata.len(),
        };
        let digest = plan_digest(&plan);
        self.engine.control_store()?.insert_plan(&plan, &digest)?;
        Ok(plan)
    }

    /// 完成计划解析、源证据确认与控制库登记。
    /// 参数：request 聚合范围、主体、动作、节点、目标及预算。
    /// 返回：已登记的不可变计划或授权、证据、存储错误。
    pub(super) fn build(&self, request: PlanRequest<'_>) -> Result<Plan, OpsError> {
        let PlanRequest {
            scope_id,
            principal,
            action,
            node_ids,
            target,
            max_bytes,
            recovery,
        } = request;
        // The scope must be live: a revoked scope never yields a plan.
        let scope = self
            .engine
            .scope(scope_id)
            .map_err(|_| OpsError::NoSuchScope(scope_id.as_str().to_owned()))?;
        if scope.revoked {
            return Err(OpsError::NotAuthorized("scope is revoked".into()));
        }
        require_action(&self.engine, scope_id, principal, action)?;
        if let Some(directory) = target {
            require_destination(&self.engine, directory, principal, action)?;
        }
        let deadline = Instant::now() + Duration::from_millis(1000);
        let revision = self
            .engine
            .latest_revision(scope_id)
            .map_err(|_| OpsError::NoSuchScope(scope_id.as_str().to_owned()))?
            .ok_or_else(|| OpsError::Stale("scope has no published revision".into()))?;
        let authorizer = self.engine.policy_authorizer()?;
        self.engine
            .authorize_revision(Some(scope_id), &revision, principal, &authorizer)?;
        Self::check_metadata_deadline(deadline)?;
        let remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        if remaining_ms == 0 {
            return Err(EngineError::Business(BusinessError::BudgetExceeded).into());
        }
        // 复用授权 reader 精确取选中行；嵌套结果保留每个 ID 的业务错误顺序。
        let resolved = self.engine.with_authorized_revision_reader(
            &revision,
            principal,
            &authorizer,
            remaining_ms,
            |reader, snapshot, _| {
                let resolved = Self::resolve_selected(reader, snapshot, node_ids, deadline);
                #[cfg(test)]
                if let Some(observer) = &self.metadata_read_observer {
                    observer();
                }
                Ok(resolved)
            },
        )?;
        // 最终授权可能等待控制锁，返回后的共同期限同样必须确认。
        Self::check_metadata_deadline(deadline)?;
        let resolved = resolved?;
        if resolved.is_empty() {
            return Err(OpsError::EmptyPlan);
        }

        // Remove parent/child overlaps: keeping both would move the same bytes
        // twice and double-count the space (OP-02).
        let resolved = drop_nested(&resolved)?;
        let planned_bytes = resolved.iter().try_fold(0_u64, |sum, item| {
            sum.checked_add(item.3)
                .ok_or_else(|| OpsError::Stale("plan byte count overflowed".into()))
        })?;
        if planned_bytes > max_bytes {
            return Err(OpsError::Stale(format!(
                "plan needs {planned_bytes} bytes, budget is {max_bytes}"
            )));
        }
        let mut evidenced: Vec<(u64, PathBuf, SourceEvidence)> = Vec::with_capacity(resolved.len());
        let mut remaining = max_bytes;
        for (node_id, path, _, _) in &resolved {
            let evidence = capture_source(path, remaining)?;
            remaining = remaining
                .checked_sub(evidence.bytes)
                .ok_or_else(|| OpsError::Stale("plan exceeded its byte budget".into()))?;
            evidenced.push((*node_id, path.clone(), evidence));
        }
        let expected_bytes = evidenced.iter().try_fold(0_u64, |sum, item| {
            sum.checked_add(item.2.bytes)
                .ok_or_else(|| OpsError::Stale("plan byte count overflowed".into()))
        })?;
        if expected_bytes > max_bytes {
            return Err(OpsError::Stale(format!(
                "plan needs {expected_bytes} bytes, budget is {max_bytes}"
            )));
        }

        let items: Vec<PlanItem> = evidenced
            .iter()
            .map(|(node_id, path, evidence)| PlanItem {
                node_id: *node_id,
                locator_key: locator_key(path),
                identity: evidence.identity.clone(),
                source_fingerprint: Some(evidence.fingerprint.clone()),
                includes_descendants: false,
                recovery_ref: None,
            })
            .collect();

        // Resolve the policy epoch in its own scope: the authorizer takes the
        // control lock, and holding it again for the plan insert would
        // self-deadlock.
        let policy_version = self.engine.policy_authorizer()?.current_version();

        let plan = Plan {
            plan_id: format!("plan-{}", uuid::Uuid::new_v4()),
            scope_id: scope_id.clone(),
            principal: principal.clone(),
            action,
            items,
            target_locator_key: target.map(locator_key),
            policy_version,
            max_bytes,
            created_at_unix_ms: now_ms(),
            expires_at_unix_ms: now_ms() + 15 * 60_000,
            recovery,
            expected_bytes,
        };
        let digest = plan_digest(&plan);
        let mut control = self.engine.control_store()?;
        control.insert_plan(&plan, &digest)?;
        Ok(plan)
    }

    /// 按请求顺序读取选中节点、原路径及 live 元数据，不物化完整图。
    /// 参数：reader/snapshot 是已授权固定快照，node_ids 是完整选择，deadline 是共同期限。
    /// 返回：全部定位或原有缺失/路径/存储错误；超时不返回部分选择。
    pub(super) fn resolve_selected(
        reader: &SqliteSnapshotStore,
        snapshot: &str,
        node_ids: &[u64],
        deadline: Instant,
    ) -> Result<Vec<Resolved>, OpsError> {
        let mut resolved = Vec::new();
        for &node_id in node_ids {
            Self::check_metadata_deadline(deadline)?;
            if node_id > i64::MAX as u64 {
                return Err(OpsError::NoSuchNode(node_id));
            }
            let node = reader
                .node(snapshot, node_id)
                .map_err(EngineError::from)?
                .ok_or(OpsError::NoSuchNode(node_id))?;
            let path = node
                .locator
                .raw_path()
                .ok_or_else(|| OpsError::Stale(format!("node {node_id} has no native path")))?;
            let live = std::fs::symlink_metadata(&path)
                .map_err(|error| OpsError::Stale(format!("node {node_id}: {error}")))?;
            let identity = identity_of(&path, &live);
            // 继续按对象自身 live 长度计费；重叠去除和目录证据仍在后续原有阶段。
            resolved.push((node_id, path, identity, live.len()));
        }
        Self::check_metadata_deadline(deadline)?;
        Ok(resolved)
    }

    /// 拒绝已过期的元数据解析，含最终授权返回后的检查。
    /// 参数：deadline 为整次图元数据阶段的固定截止时间。
    /// 返回：尚未到期为 ()，到期为预算错误；不延长期限。
    pub(super) fn check_metadata_deadline(deadline: Instant) -> Result<(), OpsError> {
        if Instant::now() >= deadline {
            return Err(EngineError::Business(BusinessError::BudgetExceeded).into());
        }
        Ok(())
    }
}
