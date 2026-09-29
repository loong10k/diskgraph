//! Control-plane records for reversible execution (P5 tasks 6.1-6.5, 6.11-6.13,
//! specs OP-02 / OP-03 / OP-08 / RT-03).
//!
//! The invariants this layer exists to enforce:
//! - A plan is immutable: created once, identified by a digest over its exact
//!   object set, target, policy version, budget, deadline, and recovery rule.
//! - An approval binds to (plan digest, principal, action) and expires.
//! - One idempotency key maps to one operation per principal; a different
//!   request under the same key is refused, never merged.
//! - An item's intent is persisted before any file side effect, so a crash
//!   leaves "intended" rather than an unrecorded mutation.

use diskgraph_core::{FileActionKind, PrincipalId, ScopeId};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{Result, StoreError};

/// One concrete object inside a plan: a resolved locator plus the identity
/// observed when the plan was created.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    /// Stable identifier (node id) inside the bound revision.
    pub node_id: u64,
    /// Lossless locator key (raw bytes, not a display string).
    pub locator_key: String,
    /// Volume + file identity captured at plan time, re-checked before apply.
    pub identity: Option<String>,
    /// True when the plan expands to descendants (directory boundary).
    pub includes_descendants: bool,
    /// The recovery record a restore plan is derived from. Absent for every
    /// action except restore.
    #[serde(default)]
    pub recovery_ref: Option<String>,
}

/// An immutable plan. Nothing here mutates after creation; `state` only moves
/// forward (validated -> expired/revoked/applied).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub plan_id: String,
    pub scope_id: ScopeId,
    pub principal: PrincipalId,
    pub action: FileActionKind,
    pub items: Vec<PlanItem>,
    /// Target directory for move/copy/restore, as a locator key.
    pub target_locator_key: Option<String>,
    pub policy_version: u64,
    pub max_bytes: u64,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    /// How a failed or completed trash can be undone.
    pub recovery: RecoveryRule,
    /// Bytes the plan expects to move, for the final budget check.
    pub expected_bytes: u64,
}

/// The only acceptable recovery stories; "delete" is deliberately absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryRule {
    /// Moved into a same-volume quarantine with a restore record.
    Quarantine,
    /// The action is reversible by moving back to the original location.
    MoveBack,
    /// Copying: the source is untouched, so nothing needs undoing.
    NoneNeeded,
}

/// Plan lifecycle. A plan is never edited, only invalidated or consumed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
    Validated,
    Expired,
    Revoked,
    Applied,
}

impl PlanState {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    fn wire_name(self) -> &'static str {
        match self {
            Self::Validated => "validated",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Applied => "applied",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "validated" => Self::Validated,
            "expired" => Self::Expired,
            "revoked" => Self::Revoked,
            "applied" => Self::Applied,
            _ => return None,
        })
    }
}

/// A trusted approval for exactly one plan digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub approval_ref: String,
    pub plan_id: String,
    pub plan_digest: String,
    pub principal: PrincipalId,
    pub action: FileActionKind,
    pub issued_by: String,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub revoked: bool,
}

/// Operation lifecycle. Distinct from the plan: an operation records real
/// side effects and can end partially.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Queued,
    Revalidating,
    Running,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
    NeedsAttention,
}

impl OperationState {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    fn wire_name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Revalidating => "revalidating",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::NeedsAttention => "needs_attention",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "revalidating" => Self::Revalidating,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "partial" => Self::Partial,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "needs_attention" => Self::NeedsAttention,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Cancelled | Self::NeedsAttention
        )
    }
}

/// Per-item progress. `intent` is written before the file changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    /// Nothing attempted yet.
    Pending,
    /// Intent recorded; the mutation may or may not have happened.
    IntentRecorded,
    /// The item completed with a recorded result.
    Done,
    /// The item failed; `detail` says why.
    Failed,
    /// The item was cancelled before it ran.
    Cancelled,
}

impl IntentState {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    fn wire_name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::IntentRecorded => "intent_recorded",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "intent_recorded" => Self::IntentRecorded,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// One item's recorded progress.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperationItem {
    pub operation_id: String,
    pub item_index: u32,
    pub intent: IntentState,
    pub result: OperationItemResult,
    pub detail: String,
    pub recovery_ref: Option<String>,
}

/// What actually happened to one item.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationItemResult {
    Pending,
    Moved,
    Copied,
    Quarantined,
    Restored,
    Skipped,
    Failed,
}

impl OperationItemResult {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    fn wire_name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Moved => "moved",
            Self::Copied => "copied",
            Self::Quarantined => "quarantined",
            Self::Restored => "restored",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "moved" => Self::Moved,
            "copied" => Self::Copied,
            "quarantined" => Self::Quarantined,
            "restored" => Self::Restored,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

/// A durable record of where a quarantined object went and how to get it back.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryEntry {
    pub recovery_ref: String,
    pub operation_id: String,
    pub scope_id: ScopeId,
    pub original_locator: String,
    pub quarantine_locator: String,
    pub identity: String,
    pub created_at_unix_ms: u64,
    pub state: RecoveryState,
}

/// Whether a recovery entry can still be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    Available,
    Restored,
    Lost,
}

impl RecoveryState {
    // Used by the ops layer as it writes states; the store keeps the codecs so
    // the wire form stays defined in exactly one place.
    #[allow(dead_code)]
    fn wire_name(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Restored => "restored",
            Self::Lost => "lost",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "available" => Self::Available,
            "restored" => Self::Restored,
            "lost" => Self::Lost,
            _ => return None,
        })
    }
}

/// An operation as stored.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub operation_id: String,
    pub plan_id: String,
    pub scope_id: ScopeId,
    pub principal: PrincipalId,
    pub idempotency_key: String,
    pub request_digest: String,
    pub state: OperationState,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

/// Milliseconds since the epoch, used for every control-plane timestamp.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn bad_state(table: &str, value: &str) -> StoreError {
    StoreError::InvalidGraph(format!("{table} has an unknown state {value:?}"))
}

impl crate::ControlStore {
    /// Persists an immutable plan and returns its digest.
    pub fn insert_plan(&mut self, plan: &Plan, digest: &str) -> Result<String> {
        let json = serde_json::to_string(plan)
            .map_err(|error| StoreError::InvalidGraph(error.to_string()))?;
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO plans (plan_id, scope_id, principal, action, plan_json, digest, created_at_unix_ms, expires_at_unix_ms, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'validated')",
                params![
                    plan.plan_id,
                    plan.scope_id.as_str(),
                    plan.principal.as_str(),
                    action_name(plan.action),
                    json,
                    digest,
                    plan.created_at_unix_ms as i64,
                    plan.expires_at_unix_ms as i64,
                ],
            )?;
            Ok(plan.plan_id.clone())
        })
    }

    /// Loads a plan by id.
    pub fn plan(&self, plan_id: &str) -> Result<Plan> {
        let json: String = self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT plan_json FROM plans WHERE plan_id = ?1",
                    [plan_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| StoreError::PlanNotFound(plan_id.to_owned()))
        })?;
        serde_json::from_str(&json).map_err(|error| StoreError::InvalidGraph(error.to_string()))
    }

    /// The digest recorded for a plan.
    pub fn plan_digest(&self, plan_id: &str) -> Result<String> {
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT digest FROM plans WHERE plan_id = ?1",
                    [plan_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| StoreError::PlanNotFound(plan_id.to_owned()))
        })
    }

    /// Current plan state, or `expired` once the deadline passed.
    pub fn plan_state(&self, plan_id: &str) -> Result<PlanState> {
        let (state, expires_at, now): (String, i64, i64) = self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT state, expires_at_unix_ms, ?2 FROM plans WHERE plan_id = ?1",
                    params![plan_id, now_ms() as i64],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?
                .ok_or_else(|| StoreError::PlanNotFound(plan_id.to_owned()))
        })?;
        let state = PlanState::parse(&state).ok_or_else(|| bad_state("plans", &state))?;
        if state == PlanState::Validated && now >= expires_at {
            return Ok(PlanState::Expired);
        }
        Ok(state)
    }

    /// Marks a plan consumed; a plan can be applied at most once.
    pub fn mark_plan_applied(&mut self, plan_id: &str) -> Result<()> {
        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE plans SET state = 'applied' WHERE plan_id = ?1 AND state = 'validated'",
                [plan_id],
            )?;
            if changed == 0 {
                return Err(StoreError::Conflict(format!(
                    "plan {plan_id} is not in a validated state"
                )));
            }
            Ok(())
        })
    }

    /// Revokes a plan before it is applied.
    pub fn revoke_plan(&mut self, plan_id: &str) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE plans SET state = 'revoked' WHERE plan_id = ?1 AND state = 'validated'",
                [plan_id],
            )?;
            Ok(())
        })
    }

    /// Records a trusted approval for one plan digest.
    pub fn insert_approval(&mut self, approval: &Approval) -> Result<String> {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO approvals (approval_ref, plan_id, plan_digest, principal, action, issued_by, issued_at_unix_ms, expires_at_unix_ms, revoked)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    approval.approval_ref,
                    approval.plan_id,
                    approval.plan_digest,
                    approval.principal.as_str(),
                    action_name(approval.action),
                    approval.issued_by,
                    approval.issued_at_unix_ms as i64,
                    approval.expires_at_unix_ms as i64,
                    approval.revoked as i64,
                ],
            )?;
            Ok(approval.approval_ref.clone())
        })
    }

    /// Looks an approval up, refusing revoked, expired, or foreign-plan uses.
    /// The approval must match the exact plan id, plan digest, principal, and
    /// action requested, so a client cannot widen a narrow approval (OP-03).
    pub fn verify_approval(
        &self,
        approval_ref: &str,
        wanted_plan_id: &str,
        wanted_plan_digest: &str,
        wanted_principal: &PrincipalId,
        wanted_action: FileActionKind,
    ) -> Result<Approval> {
        let row: Option<(String, String, String, String, i64, i64, i64)> =
            self.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT plan_id, plan_digest, principal, action, issued_at_unix_ms, expires_at_unix_ms, revoked
                         FROM approvals WHERE approval_ref = ?1",
                        [approval_ref],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                                row.get(5)?,
                                row.get(6)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(StoreError::from)
            })?;
        let (plan_id, plan_digest, principal, action, issued_at, expires_at, revoked) =
            row.ok_or_else(|| StoreError::ApprovalRequired("unknown approval".into()))?;
        if revoked != 0 {
            return Err(StoreError::ApprovalRequired("approval revoked".into()));
        }
        if plan_id != wanted_plan_id || plan_digest != wanted_plan_digest {
            return Err(StoreError::ApprovalRequired(
                "approval is bound to a different plan".into(),
            ));
        }
        if principal != wanted_principal.as_str() {
            return Err(StoreError::ApprovalRequired(
                "approval is for another principal".into(),
            ));
        }
        let action =
            action_from_name(&action).ok_or_else(|| bad_state("approvals.action", &action))?;
        if action != wanted_action {
            return Err(StoreError::ApprovalRequired(
                "approval is for another action".into(),
            ));
        }
        if now_ms() >= expires_at.max(0) as u64 {
            return Err(StoreError::ApprovalRequired("approval expired".into()));
        }
        Ok(Approval {
            approval_ref: approval_ref.to_owned(),
            plan_id,
            plan_digest,
            principal: PrincipalId::new(principal)
                .map_err(|error| StoreError::InvalidGraph(error.to_string()))?,
            action,
            issued_by: String::new(),
            issued_at_unix_ms: issued_at.max(0) as u64,
            expires_at_unix_ms: expires_at.max(0) as u64,
            revoked: false,
        })
    }

    /// Revokes an approval before it is used.
    pub fn revoke_approval(&mut self, approval_ref: &str) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE approvals SET revoked = 1 WHERE approval_ref = ?1",
                [approval_ref],
            )?;
            Ok(())
        })
    }

    /// Creates an operation, or returns the existing one for a reused
    /// idempotency key. A different request under the same key is refused
    /// (OP-08).
    pub fn begin_operation(
        &mut self,
        operation: &Operation,
        items: usize,
    ) -> Result<(String, bool)> {
        self.with_connection(|connection| {
            let existing: Option<(String, String)> = connection
                .query_row(
                    "SELECT operation_id, request_digest FROM operations
                     WHERE principal = ?1 AND idempotency_key = ?2",
                    params![operation.principal.as_str(), operation.idempotency_key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((operation_id, digest)) = existing {
                if digest != operation.request_digest {
                    return Err(StoreError::IdempotencyConflict);
                }
                return Ok((operation_id, false));
            }
            connection.execute(
                "INSERT INTO operations (operation_id, plan_id, scope_id, principal, idempotency_key, request_digest, state, created_at_unix_ms, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7, ?7)",
                params![
                    operation.operation_id,
                    operation.plan_id,
                    operation.scope_id.as_str(),
                    operation.principal.as_str(),
                    operation.idempotency_key,
                    operation.request_digest,
                    operation.created_at_unix_ms as i64,
                ],
            )?;
            for index in 0..items {
                connection.execute(
                    "INSERT INTO operation_items (operation_id, item_index, intent_state, result_state, detail, recovery_ref)
                     VALUES (?1, ?2, 'pending', 'pending', '', NULL)",
                    params![operation.operation_id, index as i64],
                )?;
            }
            Ok((operation.operation_id.clone(), true))
        })
    }

    /// The operation recorded for one principal's idempotency key, if any.
    pub fn operation_for_key(
        &self,
        principal: &PrincipalId,
        idempotency_key: &str,
    ) -> Result<Option<Operation>> {
        let id: Option<String> = self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT operation_id FROM operations
                     WHERE principal = ?1 AND idempotency_key = ?2",
                    params![principal.as_str(), idempotency_key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)
        })?;
        match id {
            Some(id) => Ok(Some(self.operation(&id)?)),
            None => Ok(None),
        }
    }

    /// Loads one operation.
    pub fn operation(&self, operation_id: &str) -> Result<Operation> {
        self.with_connection(|connection| {
            let row: Option<(String, String, String, String, String, i64, i64)> = connection
                .query_row(
                    "SELECT plan_id, scope_id, principal, idempotency_key, request_digest, created_at_unix_ms, updated_at_unix_ms
                     FROM operations WHERE operation_id = ?1",
                    [operation_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                        ))
                    },
                )
                .optional()?;
            let (plan_id, scope, principal, key, digest, created, updated) = row
                .ok_or_else(|| StoreError::OperationNotFound(operation_id.to_owned()))?;
            let state: String = connection.query_row(
                "SELECT state FROM operations WHERE operation_id = ?1",
                [operation_id],
                |row| row.get(0),
            )?;
            Ok(Operation {
                operation_id: operation_id.to_owned(),
                plan_id,
                scope_id: ScopeId::new(scope).map_err(|e| StoreError::InvalidGraph(e.to_string()))?,
                principal: PrincipalId::new(principal)
                    .map_err(|e| StoreError::InvalidGraph(e.to_string()))?,
                idempotency_key: key,
                request_digest: digest,
                state: OperationState::parse(&state).ok_or_else(|| bad_state("operations", &state))?,
                created_at_unix_ms: created.max(0) as u64,
                updated_at_unix_ms: updated.max(0) as u64,
            })
        })
    }

    /// Advances an operation's state.
    pub fn set_operation_state(&mut self, operation_id: &str, state: OperationState) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE operations SET state = ?2, updated_at_unix_ms = ?3 WHERE operation_id = ?1",
                params![operation_id, state.wire_name(), now_ms() as i64],
            )?;
            Ok(())
        })
    }

    /// Every item of an operation, in index order.
    pub fn operation_items(&self, operation_id: &str) -> Result<Vec<OperationItem>> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT item_index, intent_state, result_state, detail, recovery_ref
                 FROM operation_items WHERE operation_id = ?1 ORDER BY item_index ASC",
            )?;
            let rows = statement.query_map([operation_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?;
            let mut items = Vec::new();
            for row in rows {
                let (index, intent, result, detail, recovery_ref) = row?;
                items.push(OperationItem {
                    operation_id: operation_id.to_owned(),
                    item_index: index.max(0) as u32,
                    intent: IntentState::parse(&intent)
                        .ok_or_else(|| bad_state("operation_items.intent_state", &intent))?,
                    result: OperationItemResult::parse(&result)
                        .ok_or_else(|| bad_state("operation_items.result_state", &result))?,
                    detail,
                    recovery_ref,
                });
            }
            Ok(items)
        })
    }

    /// Records the intent for one item. This must happen before the file is
    /// touched (OP-08).
    pub fn record_intent(&mut self, operation_id: &str, index: u32) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE operation_items SET intent_state = 'intent_recorded'
                 WHERE operation_id = ?1 AND item_index = ?2 AND intent_state = 'pending'",
                params![operation_id, index as i64],
            )?;
            Ok(())
        })
    }

    /// Records the outcome of one item.
    pub fn record_item_result(
        &mut self,
        operation_id: &str,
        index: u32,
        result: OperationItemResult,
        detail: &str,
        recovery_ref: Option<&str>,
    ) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE operation_items
                 SET result_state = ?3, detail = ?4, recovery_ref = ?5
                 WHERE operation_id = ?1 AND item_index = ?2",
                params![
                    operation_id,
                    index as i64,
                    result.wire_name(),
                    detail,
                    recovery_ref,
                ],
            )?;
            Ok(())
        })
    }

    /// Records a durable recovery mapping for a quarantined object.
    pub fn insert_recovery(&mut self, entry: &RecoveryEntry) -> Result<String> {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO recovery_entries (recovery_ref, operation_id, scope_id, original_locator, quarantine_locator, identity, created_at_unix_ms, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    entry.recovery_ref,
                    entry.operation_id,
                    entry.scope_id.as_str(),
                    entry.original_locator,
                    entry.quarantine_locator,
                    entry.identity,
                    entry.created_at_unix_ms as i64,
                    entry.state.wire_name(),
                ],
            )?;
            Ok(entry.recovery_ref.clone())
        })
    }

    /// Loads one recovery entry.
    pub fn recovery(&self, recovery_ref: &str) -> Result<RecoveryEntry> {
        self.with_connection(|connection| {
            let row: Option<(String, String, String, String, String, i64, String)> = connection
                .query_row(
                    "SELECT operation_id, scope_id, original_locator, quarantine_locator, identity, created_at_unix_ms, state
                     FROM recovery_entries WHERE recovery_ref = ?1",
                    [recovery_ref],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                        ))
                    },
                )
                .optional()?;
            let (operation_id, scope, original, quarantine, identity, created, state) = row
                .ok_or_else(|| StoreError::RecoveryNotFound(recovery_ref.to_owned()))?;
            Ok(RecoveryEntry {
                recovery_ref: recovery_ref.to_owned(),
                operation_id,
                scope_id: ScopeId::new(scope).map_err(|e| StoreError::InvalidGraph(e.to_string()))?,
                original_locator: original,
                quarantine_locator: quarantine,
                identity,
                created_at_unix_ms: created.max(0) as u64,
                state: RecoveryState::parse(&state)
                    .ok_or_else(|| bad_state("recovery_entries.state", &state))?,
            })
        })
    }

    /// Marks a recovery entry as consumed by a restore.
    pub fn mark_recovery_restored(&mut self, recovery_ref: &str) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE recovery_entries SET state = 'restored' WHERE recovery_ref = ?1",
                [recovery_ref],
            )?;
            Ok(())
        })
    }

    /// Operations for a scope, newest first (C26 listing).
    pub fn list_operations(&self, scope_id: &ScopeId, limit: u64) -> Result<Vec<Operation>> {
        let ids: Vec<String> = self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT operation_id FROM operations WHERE scope_id = ?1
                 ORDER BY created_at_unix_ms DESC, operation_id DESC LIMIT ?2",
            )?;
            let rows =
                statement.query_map(params![scope_id.as_str(), limit as i64], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        })?;
        ids.iter().map(|id| self.operation(id)).collect()
    }
}

fn action_name(action: FileActionKind) -> &'static str {
    match action {
        FileActionKind::Move => "move",
        FileActionKind::Copy => "copy",
        FileActionKind::Trash => "trash",
        FileActionKind::Restore => "restore",
        FileActionKind::Purge => "purge",
    }
}

fn action_from_name(value: &str) -> Option<FileActionKind> {
    Some(match value {
        "move" => FileActionKind::Move,
        "copy" => FileActionKind::Copy,
        "trash" => FileActionKind::Trash,
        "restore" => FileActionKind::Restore,
        "purge" => FileActionKind::Purge,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(plan_id: &str) -> Plan {
        Plan {
            plan_id: plan_id.into(),
            scope_id: ScopeId::new("scope-1").unwrap(),
            principal: PrincipalId::new("agent").unwrap(),
            action: FileActionKind::Trash,
            items: vec![PlanItem {
                node_id: 1,
                locator_key: "raw-key-1".into(),
                identity: Some("dev:ino".into()),
                includes_descendants: false,
                recovery_ref: None,
            }],
            target_locator_key: None,
            policy_version: 1,
            max_bytes: 1024,
            created_at_unix_ms: now_ms(),
            expires_at_unix_ms: now_ms() + 60_000,
            recovery: RecoveryRule::Quarantine,
            expected_bytes: 512,
        }
    }

    fn operation(id: &str, key: &str, digest: &str) -> Operation {
        Operation {
            operation_id: id.into(),
            plan_id: "plan-1".into(),
            scope_id: ScopeId::new("scope-1").unwrap(),
            principal: PrincipalId::new("agent").unwrap(),
            idempotency_key: key.into(),
            request_digest: digest.into(),
            state: OperationState::Queued,
            created_at_unix_ms: now_ms(),
            updated_at_unix_ms: now_ms(),
        }
    }

    fn approval(ref_id: &str, digest: &str) -> Approval {
        Approval {
            approval_ref: ref_id.into(),
            plan_id: "plan-1".into(),
            plan_digest: digest.into(),
            principal: PrincipalId::new("agent").unwrap(),
            action: FileActionKind::Trash,
            issued_by: "admin-console".into(),
            issued_at_unix_ms: now_ms(),
            expires_at_unix_ms: now_ms() + 60_000,
            revoked: false,
        }
    }

    /// The scope every fixture plan and operation belongs to. A fixed id
    /// keeps the foreign keys satisfied without a lookup per test.
    const FIXTURE_SCOPE: &str = "scope-1";

    /// Opens a store with the fixture scope registered, so plan and operation
    /// rows satisfy their foreign keys the way they do in production.
    fn scoped_store(_label: &str) -> crate::ControlStore {
        let mut store = crate::ControlStore::open_in_memory().unwrap();
        // Drive the scope id directly so fixtures can reference it.
        store
            .insert_scope_row(
                FIXTURE_SCOPE,
                &diskgraph_core::Locator::from_native_path(std::path::Path::new("/tmp/fixture")),
            )
            .unwrap();
        store
    }

    #[test]
    fn plans_are_immutable_and_digest_addressed() {
        let mut store = scoped_store("plans");
        let plan = plan("plan-1");
        store.insert_plan(&plan, "digest-1").unwrap();
        assert_eq!(store.plan("plan-1").unwrap(), plan);
        assert_eq!(store.plan_digest("plan-1").unwrap(), "digest-1");
        assert!(matches!(
            store.insert_plan(&plan, "digest-2"),
            Err(StoreError::Sqlite(_))
        ));
        // A plan can only be consumed once.
        assert_eq!(store.plan_state("plan-1").unwrap(), PlanState::Validated);
        store.mark_plan_applied("plan-1").unwrap();
        assert_eq!(store.plan_state("plan-1").unwrap(), PlanState::Applied);
        assert!(store.mark_plan_applied("plan-1").is_err());
    }

    #[test]
    fn expired_plans_never_look_valid() {
        let mut store = scoped_store("expired");
        let mut plan = plan("plan-expired");
        plan.expires_at_unix_ms = now_ms() - 1;
        store.insert_plan(&plan, "d").unwrap();
        assert_eq!(
            store.plan_state("plan-expired").unwrap(),
            PlanState::Expired
        );
    }

    #[test]
    fn approvals_bind_to_digest_principal_and_action() {
        let mut store = scoped_store("approvals");
        store.insert_plan(&plan("plan-1"), "digest-1").unwrap();
        store
            .insert_approval(&approval("ap-1", "digest-1"))
            .unwrap();
        let principal = PrincipalId::new("agent").unwrap();
        assert!(
            store
                .verify_approval(
                    "ap-1",
                    "plan-1",
                    "digest-1",
                    &principal,
                    FileActionKind::Trash
                )
                .is_ok()
        );
        // Wrong plan digest, principal, and action are all refused.
        assert!(
            store
                .verify_approval(
                    "ap-1",
                    "plan-1",
                    "digest-other",
                    &principal,
                    FileActionKind::Trash
                )
                .is_err()
        );
        assert!(
            store
                .verify_approval(
                    "ap-1",
                    "plan-other",
                    "digest-1",
                    &principal,
                    FileActionKind::Trash
                )
                .is_err()
        );
        let other = PrincipalId::new("intruder").unwrap();
        assert!(
            store
                .verify_approval("ap-1", "plan-1", "digest-1", &other, FileActionKind::Trash)
                .is_err()
        );
        assert!(
            store
                .verify_approval(
                    "ap-1",
                    "plan-1",
                    "digest-1",
                    &principal,
                    FileActionKind::Move
                )
                .is_err()
        );
    }

    #[test]
    fn revoked_and_expired_approvals_are_refused() {
        let mut store = scoped_store("revoked");
        store.insert_plan(&plan("plan-1"), "digest-1").unwrap();
        store
            .insert_approval(&approval("ap-1", "digest-1"))
            .unwrap();
        store.revoke_approval("ap-1").unwrap();
        let principal = PrincipalId::new("agent").unwrap();
        assert!(
            store
                .verify_approval(
                    "ap-1",
                    "plan-1",
                    "digest-1",
                    &principal,
                    FileActionKind::Trash
                )
                .is_err()
        );

        let mut expired = approval("ap-2", "digest-1");
        expired.expires_at_unix_ms = now_ms() - 1;
        store.insert_approval(&expired).unwrap();
        assert!(
            store
                .verify_approval(
                    "ap-2",
                    "plan-1",
                    "digest-1",
                    &principal,
                    FileActionKind::Trash
                )
                .is_err()
        );
    }

    #[test]
    fn one_idempotency_key_maps_to_one_operation() {
        let mut store = scoped_store("idempotency");
        store.insert_plan(&plan("plan-1"), "plan-digest").unwrap();

        let (id, created) = store
            .begin_operation(&operation("op-1", "key-1", "d-1"), 2)
            .unwrap();
        assert!(created);
        assert_eq!(id, "op-1");
        // Same request, same key: the original operation is returned.
        let (again, created) = store
            .begin_operation(&operation("op-2", "key-1", "d-1"), 2)
            .unwrap();
        assert!(!created);
        assert_eq!(again, "op-1");
        // Same key, different request: refused, never merged.
        assert!(matches!(
            store.begin_operation(&operation("op-3", "key-1", "d-2"), 2),
            Err(StoreError::IdempotencyConflict)
        ));

        // A different principal may reuse the key, in its own scope.
        store
            .insert_scope_row(
                "scope-2",
                &diskgraph_core::Locator::from_native_path(std::path::Path::new("/tmp/other")),
            )
            .unwrap();
        store
            .insert_plan(
                &Plan {
                    scope_id: ScopeId::new("scope-2").unwrap(),
                    ..plan("plan-2")
                },
                "plan-digest-2",
            )
            .unwrap();
        let mut other = operation("op-4", "key-1", "d-1");
        other.principal = PrincipalId::new("other").unwrap();
        other.scope_id = ScopeId::new("scope-2").unwrap();
        other.plan_id = "plan-2".into();
        let (_, created) = store.begin_operation(&other, 1).unwrap();
        assert!(created);
    }

    #[test]
    fn intent_is_recorded_before_results() {
        let mut store = scoped_store("intent");
        store.insert_plan(&plan("plan-1"), "plan-digest").unwrap();
        store
            .begin_operation(&operation("op-1", "key-1", "d-1"), 2)
            .unwrap();
        let items = store.operation_items("op-1").unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.intent == IntentState::Pending));
        store.record_intent("op-1", 0).unwrap();
        store
            .record_item_result(
                "op-1",
                0,
                OperationItemResult::Quarantined,
                "ok",
                Some("rec-1"),
            )
            .unwrap();
        let items = store.operation_items("op-1").unwrap();
        assert_eq!(items[0].intent, IntentState::IntentRecorded);
        assert_eq!(items[0].result, OperationItemResult::Quarantined);
        assert_eq!(items[0].recovery_ref.as_deref(), Some("rec-1"));
        // The untouched item is still pending, so a crash mid-plan is visible.
        assert_eq!(items[1].intent, IntentState::Pending);
    }

    #[test]
    fn recovery_entries_survive_and_can_be_consumed_once() {
        let mut store = scoped_store("recovery");
        let entry = RecoveryEntry {
            recovery_ref: "rec-1".into(),
            operation_id: "op-1".into(),
            scope_id: ScopeId::new("scope-1").unwrap(),
            original_locator: "raw-original".into(),
            quarantine_locator: "raw-quarantine".into(),
            identity: "dev:ino".into(),
            created_at_unix_ms: now_ms(),
            state: RecoveryState::Available,
        };
        store.insert_recovery(&entry).unwrap();
        assert_eq!(store.recovery("rec-1").unwrap(), entry);
        store.mark_recovery_restored("rec-1").unwrap();
        assert_eq!(
            store.recovery("rec-1").unwrap().state,
            RecoveryState::Restored
        );
    }

    #[test]
    fn operations_are_listed_per_scope() {
        let mut store = scoped_store("listed");
        store.insert_plan(&plan("plan-1"), "plan-digest").unwrap();
        store
            .begin_operation(&operation("op-1", "k1", "d"), 1)
            .unwrap();
        let mut other_scope = operation("op-2", "k2", "d");
        other_scope.scope_id = ScopeId::new("scope-2").unwrap();
        store.begin_operation(&other_scope, 1).unwrap();
        let listed = store
            .list_operations(&ScopeId::new("scope-1").unwrap(), 10)
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].operation_id, "op-1");
    }

    #[test]
    fn action_names_round_trip() {
        for action in [
            FileActionKind::Move,
            FileActionKind::Copy,
            FileActionKind::Trash,
            FileActionKind::Restore,
            FileActionKind::Purge,
        ] {
            assert_eq!(action_from_name(action_name(action)), Some(action));
        }
    }
}
