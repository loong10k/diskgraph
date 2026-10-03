//! executor_apply：既有文件操作职责的原生 Rust 实现。
use crate::apply_outcome::ApplyOutcome;
use crate::apply_request::ApplyRequest;
use crate::executor::Executor;
use crate::fault_point::FaultPoint;
use crate::ops_error::OpsError;
use crate::ops_time::now_ms;
use crate::plan_digest::apply_request_digest;
use crate::plan_digest::plan_digest;
use diskgraph_core::FileActionKind;
use diskgraph_store::StoreError;

impl Executor {
    /// 按原有批准、幂等和意图记录顺序执行计划。
    /// 参数：request 包含计划、批准引用、幂等键及可选故障。
    /// 返回：新操作或原有幂等结果，保留部分失败与需关注终态。
    /// Applies a plan under a trusted approval.
    pub fn apply(&self, request: ApplyRequest<'_>) -> Result<ApplyOutcome, OpsError> {
        // 先在同一有限控制库 guard 中解析幂等请求；已接纳请求复用原操作，随后释放 guard。
        // 0. A retry of a request we already accepted answers with the
        //    original operation, whatever happened to the plan since. This is
        //    checked first: a client that lost the response must not be told
        //    the plan is stale when its own work already succeeded.
        {
            // One guard for the whole probe: the plan, the key lookup, and the
            // digest all read the same control database.
            let control = self.engine.control_store()?;
            let probe_plan = control.plan(request.plan_id)?;
            if let Some(existing) =
                control.operation_for_key(&probe_plan.principal, request.idempotency_key)?
            {
                if existing.request_digest
                    != apply_request_digest(&probe_plan, request.approval_ref)
                {
                    return Err(StoreError::IdempotencyConflict.into());
                }
                return Ok(ApplyOutcome {
                    operation_id: existing.operation_id,
                    started: false,
                    state: existing.state,
                    moved: 0,
                    failed: 0,
                    bytes: 0,
                });
            }
        }

        // 1. The plan must exist, still be live, and match the bound digest.
        // 2. The approval must verify for this exact plan, principal and
        //    action. An agent can never assert its own approval.
        // Each block below takes the control lock for as short as possible:
        // holding one guard across a helper that locks again would deadlock.
        let plan = {
            let control = self.engine.control_store()?;
            let plan = control.plan(request.plan_id)?;
            let digest = control.plan_digest(request.plan_id)?;
            if plan_digest(&plan) != digest {
                return Err(OpsError::Stale("plan digest drifted".into()));
            }
            if control.plan_state(request.plan_id)? != diskgraph_store::PlanState::Validated {
                return Err(OpsError::Stale(format!(
                    "plan {} is not validated",
                    request.plan_id
                )));
            }
            // The verified approval record is kept so a purge can prove its
            // approval came from the configured authority, never from the
            // agent or an ordinary operation surface (OP-07).
            let approval = control
                .verify_approval(
                    request.approval_ref,
                    &plan.plan_id,
                    &digest,
                    &plan.principal,
                    plan.action,
                )
                .map_err(|error| OpsError::NotAuthorized(error.to_string()))?;
            if plan.action == FileActionKind::Purge {
                let authority = self.purge_authority.as_deref().ok_or_else(|| {
                    OpsError::NotAuthorized(
                        "purge is irreversible and no purge authority is configured".into(),
                    )
                })?;
                if approval.issued_by != authority {
                    return Err(OpsError::NotAuthorized(
                        "a purge may only be approved by the configured purge authority".into(),
                    ));
                }
            }
            plan
        };
        self.check_plan_authorization(&plan)?;

        // 3. Revalidate every precondition against the live filesystem before
        // touching anything. The control store claims overlapping paths in
        // the same transaction that creates the operation.
        let items = self.resolve_live_items(&plan)?;

        // 4. Idempotency: one key, one operation.
        let request_digest = apply_request_digest(&plan, request.approval_ref);
        let operation = diskgraph_store::Operation {
            operation_id: format!("op-{}", uuid::Uuid::new_v4()),
            plan_id: plan.plan_id.clone(),
            scope_id: plan.scope_id.clone(),
            principal: plan.principal.clone(),
            idempotency_key: request.idempotency_key.to_owned(),
            request_digest,
            state: diskgraph_store::OperationState::Queued,
            created_at_unix_ms: now_ms(),
            updated_at_unix_ms: now_ms(),
        };
        let (operation_id, created) = {
            let mut control = self.engine.control_store()?;
            control
                .begin_operation(&operation, items.len())
                .map_err(|error| match error {
                    StoreError::Conflict(message) => OpsError::Conflict(message),
                    other => OpsError::Store(other),
                })?
        };
        if !created {
            // A retry of the same request: return the original operation and
            // do not move anything a second time.
            let existing = self.engine.control_store()?.operation(&operation_id)?;
            return Ok(ApplyOutcome {
                operation_id,
                started: false,
                state: existing.state,
                moved: 0,
                failed: 0,
                bytes: 0,
            });
        }

        // 5. Execute item by item, recording intent before each mutation.
        self.engine.control_store()?.advance_operation_state(
            &operation_id,
            diskgraph_store::OperationState::Revalidating,
        )?;
        let mut moved = 0usize;
        let mut failed = 0usize;
        let mut bytes = 0u64;
        for (index, item) in items.iter().enumerate() {
            let index = index as u32;
            // A long batch must stop at the first withdrawn approval or
            // action grant. No later item may be retried under old authority.
            if let Err(error) = self.check_plan_authorization(&plan).and_then(|()| {
                self.engine
                    .control_store()?
                    .verify_approval(
                        request.approval_ref,
                        &plan.plan_id,
                        &plan_digest(&plan),
                        &plan.principal,
                        plan.action,
                    )
                    .map(|_| ())
                    .map_err(|error| OpsError::NotAuthorized(error.to_string()))
            }) {
                let detail = error.to_string();
                let mut control = self.engine.control_store()?;
                for remaining in index..items.len() as u32 {
                    control.record_item_result(
                        &operation_id,
                        remaining,
                        diskgraph_store::OperationItemResult::Failed,
                        &format!("blocked before execution: {detail}"),
                        None,
                    )?;
                    failed += 1;
                }
                break;
            }
            self.engine
                .control_store()?
                .record_intent(&operation_id, index)?;
            if request.fault == Some(FaultPoint::AfterIntent) {
                // The intent is durable and the file is untouched: a later
                // attempt must reconcile rather than replay blindly.
                let actual = self.engine.control_store()?.advance_operation_state(
                    &operation_id,
                    diskgraph_store::OperationState::NeedsAttention,
                )?;
                return Ok(ApplyOutcome {
                    operation_id,
                    started: true,
                    state: actual,
                    moved,
                    failed,
                    bytes,
                });
            }
            match self.perform(
                &plan,
                item,
                request.fault,
                request.approval_ref,
                &operation_id,
            ) {
                Ok(result) => {
                    bytes += result.bytes;
                    if request.fault == Some(FaultPoint::AfterFileChange) {
                        // The file moved but the result was never recorded: the
                        // operation parks for a human instead of replaying.
                        let actual = self.engine.control_store()?.advance_operation_state(
                            &operation_id,
                            diskgraph_store::OperationState::NeedsAttention,
                        )?;
                        return Ok(ApplyOutcome {
                            operation_id,
                            started: true,
                            state: actual,
                            moved,
                            failed,
                            bytes,
                        });
                    }
                    self.engine.control_store()?.record_item_result(
                        &operation_id,
                        index,
                        result.kind,
                        &result.detail,
                        result.recovery_ref.as_deref(),
                    )?;
                    if result.kind == diskgraph_store::OperationItemResult::Failed {
                        failed += 1;
                    } else {
                        moved += 1;
                    }
                }
                Err(OpsError::ParkNeedsAttention(detail)) => {
                    // The step got part-way (for example a cross-volume move
                    // published its verified copy): park for reconciliation
                    // instead of replaying an irreversible remainder (OP-08).
                    let actual = self.engine.control_store()?.advance_operation_state(
                        &operation_id,
                        diskgraph_store::OperationState::NeedsAttention,
                    )?;
                    self.engine.control_store()?.record_item_result(
                        &operation_id,
                        index,
                        diskgraph_store::OperationItemResult::Pending,
                        &format!("parked mid-step: {detail}"),
                        None,
                    )?;
                    return Ok(ApplyOutcome {
                        operation_id,
                        started: true,
                        state: actual,
                        moved,
                        failed,
                        bytes,
                    });
                }
                Err(error) => {
                    failed += 1;
                    self.engine.control_store()?.record_item_result(
                        &operation_id,
                        index,
                        diskgraph_store::OperationItemResult::Failed,
                        &error.to_string(),
                        None,
                    )?;
                }
            }
        }

        // 6. Close out honestly: partial when some items failed.
        let state = if failed == 0 {
            diskgraph_store::OperationState::Succeeded
        } else if moved == 0 {
            diskgraph_store::OperationState::Failed
        } else {
            diskgraph_store::OperationState::Partial
        };
        let state = {
            let mut control = self.engine.control_store()?;
            let actual = control.advance_operation_state(&operation_id, state)?;
            if actual == diskgraph_store::OperationState::Succeeded {
                control.mark_plan_applied(&plan.plan_id)?;
            }
            actual
        };
        Ok(ApplyOutcome {
            operation_id,
            started: true,
            state,
            moved,
            failed,
            bytes,
        })
    }
}
