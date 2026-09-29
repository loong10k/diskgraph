//! Business error taxonomy and CLI exit codes (P0 task 1.5, specs CMD-02 / Q-01).
//!
//! The mapping below is the contract from `docs/command-reference.md` §8.
//! CLI, MCP, and FFI must report the same business codes; a bounded query that
//! truncates normally stays `ok` + `truncated` and never becomes `partial`.

/// Target business results, each with a fixed CLI exit code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusinessError {
    /// Exit 2: malformed arguments, or an ambiguous reference needing a qualifier.
    InvalidArgument,
    /// Exit 2: several objects match; the caller must qualify.
    Ambiguous,
    /// Exit 3: the principal lacks the required capability.
    PermissionDenied,
    /// Exit 3: an action needs a trusted approval that is absent or expired.
    ApprovalRequired,
    /// Exit 4: the scope has no published index; scanning is never implicit.
    NotIndexed,
    /// Exit 4: the referenced object does not exist in the bound revision.
    NotFound,
    /// Exit 5: the plan no longer matches live state; re-plan required.
    StalePlan,
    /// Exit 5: the bound revision was retired; re-query against latest.
    RevisionExpired,
    /// Exit 5: the two snapshots cannot be compared (volume, settings, coverage).
    IncompatibleHistory,
    /// Exit 6: platform or provider cannot support the requested capability.
    Unsupported,
    /// Exit 6: an optional dependency (Git, Docker, ...) is missing right now.
    Unavailable,
    /// Exit 7: a size, node, edge, or byte budget was exceeded.
    BudgetExceeded,
    /// Exit 7: the call deadline elapsed; a started job may still finish.
    Timeout,
    /// Exit 7: capacity, rate, or per-principal job limits are exhausted.
    ResourceExhausted,
    /// Exit 8: work ended incompletely with per-item results recorded.
    Partial,
    /// Exit 8: a crash or interruption left state that must be reconciled by a human.
    NeedsAttention,
    /// Exit 9: a resource, plan, or lock conflict with concurrent work.
    Conflict,
    /// Exit 9: the idempotency key was reused with a different request.
    IdempotencyConflict,
    /// Exit 10: unrecoverable server fault; only a redacted diagnostic ID is exposed.
    InternalError,
}

impl BusinessError {
    /// Stable snake_case code carried in every envelope and MCP result.
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::Ambiguous => "ambiguous",
            Self::PermissionDenied => "permission_denied",
            Self::ApprovalRequired => "approval_required",
            Self::NotIndexed => "not_indexed",
            Self::NotFound => "not_found",
            Self::StalePlan => "stale_plan",
            Self::RevisionExpired => "revision_expired",
            Self::IncompatibleHistory => "incompatible_history",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::BudgetExceeded => "budget_exceeded",
            Self::Timeout => "timeout",
            Self::ResourceExhausted => "resource_exhausted",
            Self::Partial => "partial",
            Self::NeedsAttention => "needs_attention",
            Self::Conflict => "conflict",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::InternalError => "internal_error",
        }
    }

    /// Fixed CLI exit code; entry points must agree on this table.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::InvalidArgument | Self::Ambiguous => 2,
            Self::PermissionDenied | Self::ApprovalRequired => 3,
            Self::NotIndexed | Self::NotFound => 4,
            Self::StalePlan | Self::RevisionExpired | Self::IncompatibleHistory => 5,
            Self::Unsupported | Self::Unavailable => 6,
            Self::BudgetExceeded | Self::Timeout | Self::ResourceExhausted => 7,
            Self::Partial | Self::NeedsAttention => 8,
            Self::Conflict | Self::IdempotencyConflict => 9,
            Self::InternalError => 10,
        }
    }
}

impl core::fmt::Display for BusinessError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} (exit {})", self.code(), self.exit_code())
    }
}

impl std::error::Error for BusinessError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full table from docs/command-reference.md §8; any drift here must be
    /// a deliberate contract change, never a side effect of refactoring.
    #[test]
    fn error_codes_match_the_command_reference_exit_table() {
        let expected: &[(BusinessError, &str, u8)] = &[
            (BusinessError::InvalidArgument, "invalid_argument", 2),
            (BusinessError::Ambiguous, "ambiguous", 2),
            (BusinessError::PermissionDenied, "permission_denied", 3),
            (BusinessError::ApprovalRequired, "approval_required", 3),
            (BusinessError::NotIndexed, "not_indexed", 4),
            (BusinessError::NotFound, "not_found", 4),
            (BusinessError::StalePlan, "stale_plan", 5),
            (BusinessError::RevisionExpired, "revision_expired", 5),
            (
                BusinessError::IncompatibleHistory,
                "incompatible_history",
                5,
            ),
            (BusinessError::Unsupported, "unsupported", 6),
            (BusinessError::Unavailable, "unavailable", 6),
            (BusinessError::BudgetExceeded, "budget_exceeded", 7),
            (BusinessError::Timeout, "timeout", 7),
            (BusinessError::ResourceExhausted, "resource_exhausted", 7),
            (BusinessError::Partial, "partial", 8),
            (BusinessError::NeedsAttention, "needs_attention", 8),
            (BusinessError::Conflict, "conflict", 9),
            (
                BusinessError::IdempotencyConflict,
                "idempotency_conflict",
                9,
            ),
            (BusinessError::InternalError, "internal_error", 10),
        ];
        assert_eq!(expected.len(), 19);
        for (error, code, exit) in expected {
            assert_eq!(error.code(), *code);
            assert_eq!(error.exit_code(), *exit);
        }
    }
}
