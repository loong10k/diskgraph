//! Query budgets and paging cursors (P2 task 3.11, specs Q-02 / Q-07).
//!
//! Traversal is bounded before it starts: depth, node, edge, and byte caps
//! plus a deadline. Exceeding a cap returns a normal `truncated` result with a
//! reason and a resumable cursor — never a partial failure and never an
//! unbounded expansion.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::errors::BusinessError;

/// The initial engineering ceilings from the technical design. They are caps to
/// be calibrated by benchmarks, not performance promises.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryBudget {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_response_bytes: usize,
    pub deadline_ms: u64,
}

impl Default for QueryBudget {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_nodes: 100,
            max_edges: 300,
            max_response_bytes: 64 * 1024,
            deadline_ms: 1_000,
        }
    }
}

impl QueryBudget {
    /// Refuses budgets with a zero cap; a zero limit cannot produce a result.
    pub fn validated(self) -> Result<Self, BusinessError> {
        if self.max_depth == 0
            || self.max_nodes == 0
            || self.max_edges == 0
            || self.max_response_bytes == 0
            || self.deadline_ms == 0
        {
            return Err(BusinessError::InvalidArgument);
        }
        Ok(self)
    }
}

/// Why a bounded result stopped early.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncationReason {
    DepthLimit,
    NodeLimit,
    EdgeLimit,
    ByteLimit,
    Deadline,
}

impl TruncationReason {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::DepthLimit => "depth_limit",
            Self::NodeLimit => "node_limit",
            Self::EdgeLimit => "edge_limit",
            Self::ByteLimit => "byte_limit",
            Self::Deadline => "deadline",
        }
    }
}

/// Tracks consumption while a bounded query runs.
pub struct BudgetTracker {
    budget: QueryBudget,
    started_at: std::time::Instant,
    nodes: usize,
    edges: usize,
    bytes: usize,
    truncated: Option<TruncationReason>,
}

impl BudgetTracker {
    pub fn new(budget: QueryBudget) -> Result<Self, BusinessError> {
        Ok(Self {
            budget: budget.validated()?,
            started_at: std::time::Instant::now(),
            nodes: 0,
            edges: 0,
            bytes: 0,
            truncated: None,
        })
    }

    pub fn budget(&self) -> QueryBudget {
        self.budget
    }

    /// Charges one node; returns false once the node cap is reached.
    pub fn charge_node(&mut self) -> bool {
        if self.expired() {
            return false;
        }
        if self.nodes >= self.budget.max_nodes {
            self.truncated = Some(TruncationReason::NodeLimit);
            return false;
        }
        self.nodes += 1;
        true
    }

    /// Charges one edge; returns false once the edge cap is reached.
    pub fn charge_edge(&mut self) -> bool {
        if self.expired() {
            return false;
        }
        if self.edges >= self.budget.max_edges {
            self.truncated = Some(TruncationReason::EdgeLimit);
            return false;
        }
        self.edges += 1;
        true
    }

    /// 将估计的响应字节增量计入额度；来源：DiskGraph 原生查询预算，无 Java 对应实现。
    /// 参数：bytes 为本次增量。返回：可表示且未超限为 true；否则标记 ByteLimit 并保留累计量。
    pub fn charge_bytes(&mut self, bytes: usize) -> bool {
        if self.expired() {
            return false;
        }
        // 溢出不能饱和成合法的 usize::MAX 额度，失败后整个 tracker 保持截断。
        let Some(total) = self
            .bytes
            .checked_add(bytes)
            .filter(|total| *total <= self.budget.max_response_bytes)
        else {
            self.truncated = Some(TruncationReason::ByteLimit);
            return false;
        };
        self.bytes = total;
        true
    }

    /// Depth beyond the cap stops expansion (breadth stays as-is).
    pub fn allows_depth(&mut self, depth: usize) -> bool {
        if depth > self.budget.max_depth {
            self.truncated = Some(TruncationReason::DepthLimit);
            return false;
        }
        true
    }

    pub fn nodes(&self) -> usize {
        self.nodes
    }

    pub fn edges(&self) -> usize {
        self.edges
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn truncated(&self) -> Option<TruncationReason> {
        self.truncated
    }

    /// True while the deadline has not passed; sets truncation once it has.
    fn expired(&mut self) -> bool {
        if self.truncated.is_some() {
            return true;
        }
        if self.started_at.elapsed().as_millis() as u64 >= self.budget.deadline_ms {
            self.truncated = Some(TruncationReason::Deadline);
            return true;
        }
        false
    }
}

/// A keyset cursor bound to the principal's authorization context, revision,
/// filter, and sort order. A cursor cannot be reused across a different scope,
/// revision, or filter set: cross-reuse fails instead of leaking rows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PagingCursor {
    /// Stable identity of the authorization context that issued the cursor.
    pub principal_binding: String,
    pub scope_id: String,
    pub revision_id: String,
    /// Canonical form of the filter set; changing filters invalidates the cursor.
    pub filter_binding: String,
    pub sort_binding: String,
    /// The policy epoch the cursor was issued under.
    pub policy_version: u64,
    /// The exclusive starting offset (keyset position).
    pub offset: u64,
}

/// Where a cursor must agree to be reusable. The policy version binds a
/// cursor to the authorization epoch it was issued under: after a policy
/// update, cursors from before it are refused, so a page started under old
/// grants cannot silently continue under new ones (P4 task 5.9).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CursorContext<'a> {
    pub principal_binding: &'a str,
    pub scope_id: &'a str,
    pub revision_id: &'a str,
    pub filter_binding: &'a str,
    pub sort_binding: &'a str,
    pub policy_version: u64,
}

/// Why a cursor was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorRejection {
    Malformed,
    BindingMismatch,
}

impl CursorRejection {
    pub fn business_error(self) -> BusinessError {
        match self {
            Self::Malformed | Self::BindingMismatch => BusinessError::InvalidArgument,
        }
    }
}

impl PagingCursor {
    /// Issues a cursor bound to the current context.
    pub fn issue(
        context: &CursorContext<'_>,
        filter_binding: impl Into<String>,
        sort_binding: impl Into<String>,
        offset: u64,
    ) -> Self {
        Self {
            principal_binding: context.principal_binding.to_owned(),
            scope_id: context.scope_id.to_owned(),
            revision_id: context.revision_id.to_owned(),
            filter_binding: filter_binding.into(),
            sort_binding: sort_binding.into(),
            policy_version: context.policy_version,
            offset,
        }
    }

    /// Validates the cursor against the context it would be used in.
    pub fn verify(&self, context: &CursorContext<'_>) -> Result<u64, CursorRejection> {
        if self.principal_binding != context.principal_binding
            || self.scope_id != context.scope_id
            || self.revision_id != context.revision_id
            || self.filter_binding != context.filter_binding
            || self.sort_binding != context.sort_binding
            || self.policy_version != context.policy_version
        {
            return Err(CursorRejection::BindingMismatch);
        }
        Ok(self.offset)
    }

    /// Serializes to the opaque string carried in the envelope.
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes())
    }

    /// Decodes a cursor string; corrupt input is rejected, not defaulted.
    pub fn decode(encoded: &str) -> Result<Self, CursorRejection> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .map_err(|_| CursorRejection::Malformed)?;
        serde_json::from_slice(&bytes).map_err(|_| CursorRejection::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> CursorContext<'static> {
        CursorContext {
            principal_binding: "agent@scope-1",
            scope_id: "scope-1",
            revision_id: "rev-1",
            filter_binding: "name:cargo",
            sort_binding: "size_desc,name_asc",
            policy_version: 3,
        }
    }

    #[test]
    fn budgets_stop_at_node_edge_byte_and_depth_caps() {
        let budget = QueryBudget {
            max_depth: 1,
            max_nodes: 2,
            max_edges: 1,
            max_response_bytes: 16,
            deadline_ms: 60_000,
        };
        let mut tracker = BudgetTracker::new(budget).unwrap();
        assert!(tracker.allows_depth(1));
        assert!(!tracker.allows_depth(2));
        assert_eq!(tracker.truncated(), Some(TruncationReason::DepthLimit));

        let mut tracker = BudgetTracker::new(budget).unwrap();
        assert!(tracker.charge_node());
        assert!(tracker.charge_node());
        assert!(!tracker.charge_node());
        assert_eq!(tracker.truncated(), Some(TruncationReason::NodeLimit));

        let mut tracker = BudgetTracker::new(budget).unwrap();
        assert!(tracker.charge_edge());
        assert!(!tracker.charge_edge());
        assert_eq!(tracker.truncated(), Some(TruncationReason::EdgeLimit));

        let mut tracker = BudgetTracker::new(budget).unwrap();
        assert!(tracker.charge_bytes(8));
        assert!(!tracker.charge_bytes(16));
        assert_eq!(tracker.truncated(), Some(TruncationReason::ByteLimit));
    }

    #[test]
    fn a_zero_budget_is_an_argument_error_not_a_silent_empty_result() {
        let budget = QueryBudget {
            max_nodes: 0,
            ..QueryBudget::default()
        };
        assert!(matches!(
            BudgetTracker::new(budget),
            Err(BusinessError::InvalidArgument)
        ));
    }

    #[test]
    fn response_byte_overflow_refuses_without_changing_consumption() {
        let mut tracker = BudgetTracker::new(QueryBudget::default()).unwrap();
        assert!(tracker.charge_bytes(1));
        // 非可信库参数无需实际分配巨大缓冲区，也不能使累计账本回绕。
        assert!(!tracker.charge_bytes(usize::MAX));
        assert_eq!(tracker.bytes(), 1);
        assert_eq!(tracker.truncated(), Some(TruncationReason::ByteLimit));
        assert!(!tracker.charge_bytes(0));
        assert!(!tracker.charge_node());
    }

    #[test]
    fn maximum_representable_byte_cap_is_exact_and_cannot_saturate_overflow() {
        let mut tracker = BudgetTracker::new(QueryBudget {
            max_response_bytes: usize::MAX,
            ..QueryBudget::default()
        })
        .unwrap();
        assert!(tracker.charge_bytes(usize::MAX - 1));
        assert!(tracker.charge_bytes(1));
        assert_eq!(tracker.bytes(), usize::MAX);
        assert_eq!(tracker.truncated(), None);
        assert!(!tracker.charge_bytes(1));
        assert_eq!(tracker.bytes(), usize::MAX);
        assert_eq!(tracker.truncated(), Some(TruncationReason::ByteLimit));
    }

    #[test]
    fn cursors_round_trip_and_refuse_cross_context_reuse() {
        let context = context();
        let cursor =
            PagingCursor::issue(&context, context.filter_binding, context.sort_binding, 50);
        let encoded = cursor.encode();
        let decoded = PagingCursor::decode(&encoded).unwrap();
        assert_eq!(decoded, cursor);
        assert_eq!(decoded.verify(&context), Ok(50));

        // A different principal, scope, revision, or filter refuses the cursor.
        for mutation in [
            CursorContext {
                principal_binding: "other@scope-1",
                ..context.clone()
            },
            CursorContext {
                scope_id: "scope-2",
                ..context.clone()
            },
            CursorContext {
                revision_id: "rev-2",
                ..context.clone()
            },
            CursorContext {
                filter_binding: "name:other",
                ..context.clone()
            },
        ] {
            assert_eq!(
                decoded.verify(&mutation),
                Err(CursorRejection::BindingMismatch)
            );
        }
    }

    #[test]
    fn corrupt_cursor_strings_are_rejected_not_guessed() {
        assert_eq!(
            PagingCursor::decode("!!!not-base64!!!"),
            Err(CursorRejection::Malformed)
        );
        let valid = PagingCursor::issue(&context(), "f", "s", 0).encode();
        let tampered = format!("{valid}AA");
        assert!(PagingCursor::decode(&tampered).is_err());
    }
}
