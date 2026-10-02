//! Promoting a metadata-only comparison to a content one, one file at a
//! time.
//!
//! A comparison that stops at size and timestamp answers "are these the same
//! file?" with a weaker claim than a reader usually takes it for. Two files
//! of identical length edited in place compare the same, and a caller that
//! skips a copy on that row ships a stale file. Closing the gap means reading
//! both, which is expensive and is a separate grant, so it is opt-in and
//! bounded on both axes.
//!
//! Two rules make the result trustworthy. The comparison only reads a file
//! the metadata pass already called the same, so a difference is never
//! re-decided by a different test. And a file that could not be read (a cloud
//! placeholder, a revoked grant, bytes that moved underneath the read) is
//! reported as unverified rather than as identical, because "I could not
//! check" and "they match" are different sentences.

use std::path::PathBuf;

use diskgraph_core::{Authorizer, PrincipalId, ResourceLocator, ScopeId, Verdict};

use crate::VerifyLimits;
use crate::content::{ConservativeProbe, DigestOutcome, InspectionRequest};
use crate::{ComparisonReport, Engine, EngineError};

/// How far a content verification may go.
#[derive(Clone, Copy, Debug)]
pub struct VerifyBudget {
    /// Files whose contents may be read in one comparison.
    pub max_files: u64,
    /// Bytes one file may be read for. A file larger than this is left
    /// unverified: hashing it would cost more than re-copying it.
    pub max_bytes_per_file: u64,
}

impl Default for VerifyBudget {
    fn default() -> Self {
        Self {
            // Small on purpose. A verification is for the handful of files a
            // caller is about to act on, not for re-hashing a tree.
            max_files: 256,
            max_bytes_per_file: 64 << 20,
        }
    }
}

/// What a verification pass did, and what it could not do.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct VerifySummary {
    /// 已尝试核验的文件对，任一侧失败也消耗文件预算。
    pub attempted_files: u64,
    /// Files whose contents were read on both sides and matched.
    pub confirmed_same: u64,
    /// Files whose contents were read and differ.
    pub confirmed_different: u64,
    /// Files left unverified: over the budget, unreadable, or a placeholder.
    pub unverified: u64,
    /// Bytes read across every file, on both sides.
    pub bytes_read: u64,
}

impl VerifySummary {
    /// Whether anything was left unsaid, which a caller must not mistake for
    /// "everything checked out".
    pub fn is_complete(&self) -> bool {
        self.unverified == 0
    }
}

/// Verifies the rows a comparison called the same, reading contents.
///
/// Takes the report by value and returns it with the promoted rows: the
/// caller gets one answer, and a row that was already a difference keeps the
/// verdict that produced it.
pub fn verify_same_rows(
    engine: &Engine,
    report: ComparisonReport,
    left_scope: &ScopeId,
    right_scope: &ScopeId,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    budget: VerifyBudget,
) -> Result<(ComparisonReport, VerifySummary), EngineError> {
    verify_same_rows_with_limits(
        engine,
        report,
        left_scope,
        right_scope,
        principal,
        authorizer,
        budget,
        &VerifyLimits::default(),
    )
}

/// 使用累计字节、绝对期限与取消信号核验；已有 API 使用保守默认上限。
#[allow(clippy::too_many_arguments)] // 比较上下文与共享预算是独立必需参数。
pub fn verify_same_rows_with_limits(
    engine: &Engine,
    report: ComparisonReport,
    left_scope: &ScopeId,
    right_scope: &ScopeId,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    budget: VerifyBudget,
    limits: &VerifyLimits,
) -> Result<(ComparisonReport, VerifySummary), EngineError> {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(limits.max_duration_ms);
    // The paths come from the roots the report recorded, not from the scope
    // ids: a scope id is an identifier, and joining a relative path onto one
    // would produce a path the engine's containment check rightly refuses.
    let left_root = native_root(&report.left_root)?;
    let right_root = native_root(&report.right_root)?;
    let mut summary = VerifySummary::default();
    let mut report = report;
    let mut rows = Vec::with_capacity(report.rows.len());
    for row in std::mem::take(&mut report.rows) {
        // Only the rows metadata already called the same. A difference has
        // already been decided, and re-deciding it with a costlier test would
        // make the two paths disagree about the same pair of files.
        // A directory is not hashed: its verdict comes from what it holds,
        // and asking for its contents fails, which would report every
        // directory in the tree as unverified and bury the files that
        // actually could not be read.
        let is_file = row.is_file;
        let promotable = is_file
            && matches!(row.verdict, Verdict::Same { .. })
            && row
                .left_bytes
                .is_some_and(|bytes| bytes <= budget.max_bytes_per_file);
        let out_of_budget = summary.attempted_files >= budget.max_files
            || summary.bytes_read >= limits.max_total_bytes
            || std::time::Instant::now() >= deadline
            || limits
                .cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed));
        if !promotable || out_of_budget {
            // A row that was already a difference has an answer and needs no
            // verification. A row that was "same" and did not get verified -
            // over budget, over the per-file ceiling, or unreadable - has no
            // answer beyond the metadata one, and the summary must say so
            // rather than let it read as settled.
            if is_file && matches!(row.verdict, Verdict::Same { .. }) {
                summary.unverified += 1;
            }
            rows.push(row);
            continue;
        }
        let left_path = left_root.join(&row.path);
        let right_path = right_root.join(&row.path);
        summary.attempted_files += 1;
        let left = digest_of(
            engine,
            left_scope,
            principal,
            authorizer,
            &left_path,
            budget
                .max_bytes_per_file
                .min(limits.max_total_bytes.saturating_sub(summary.bytes_read)),
            limits,
            deadline,
        );
        summary.bytes_read = summary.bytes_read.saturating_add(left.bytes_read);
        let right = digest_of(
            engine,
            right_scope,
            principal,
            authorizer,
            &right_path,
            budget
                .max_bytes_per_file
                .min(limits.max_total_bytes.saturating_sub(summary.bytes_read)),
            limits,
            deadline,
        );
        summary.bytes_read = summary.bytes_read.saturating_add(right.bytes_read);
        match (left.digest, right.digest) {
            (Some(left), Some(right)) => {
                let mut row = row;
                // The values travel with the row, not just the conclusion: a
                // caller comparing against a manifest elsewhere needs the
                // hash, and a caller that trusts the verdict has to be able to
                // see which bytes were hashed to reach it.
                row.digests = Some((left.clone(), right.clone()));
                if left == right {
                    summary.confirmed_same += 1;
                    row.verdict = Verdict::Same {
                        evidence: diskgraph_core::Evidence::Content,
                    };
                } else {
                    summary.confirmed_different += 1;
                    row.verdict = Verdict::Different {
                        reason: diskgraph_core::DifferentReason::Content,
                    };
                }
                rows.push(row);
            }
            _ => {
                // A file neither side could be read says nothing about whether
                // the two match, so the row keeps its metadata verdict and the
                // summary records that the claim was never upgraded.
                summary.unverified += 1;
                rows.push(row);
            }
        }
    }
    report.rows = rows;
    Ok((report, summary))
}

/// The filesystem path behind a locator. A comparison is between two
/// directories on this machine, so a document URI is not something to read.
fn native_root(locator: &ResourceLocator) -> Result<PathBuf, EngineError> {
    match locator {
        ResourceLocator::NativePath(path) => Ok(PathBuf::from(path)),
        ResourceLocator::DocumentUri(_) => Err(EngineError::Business(
            diskgraph_core::BusinessError::InvalidArgument,
        )),
    }
}

/// One side's digest, or nothing when it could not be read.
struct OneDigest {
    digest: Option<String>,
    bytes_read: u64,
}

#[allow(clippy::too_many_arguments)]
fn digest_of(
    engine: &Engine,
    scope: &ScopeId,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    path: &PathBuf,
    max_bytes: u64,
    limits: &VerifyLimits,
    deadline: std::time::Instant,
) -> OneDigest {
    if max_bytes == 0 || std::time::Instant::now() >= deadline {
        return OneDigest {
            digest: None,
            bytes_read: 0,
        };
    }
    let request = InspectionRequest {
        scope_id: scope,
        principal,
        path,
        offset: 0,
        max_bytes,
        cancel: limits.cancel.as_deref(),
        chunk_bytes: 1 << 20,
    };
    let outcome: DigestOutcome =
        match engine.digest_bounded_until(&request, &ConservativeProbe, authorizer, deadline) {
            Ok(outcome) => outcome,
            Err(_) => {
                return OneDigest {
                    digest: None,
                    bytes_read: 0,
                };
            }
        };
    // A digest that stopped is not a digest: the bytes were not all read, so
    // the hash says nothing about the file.
    OneDigest {
        digest: (outcome.confirmed() && !outcome.digest_hex.is_empty())
            .then_some(outcome.digest_hex),
        bytes_read: outcome.bytes_digested,
    }
}
