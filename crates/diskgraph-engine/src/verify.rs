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
        let promotable = matches!(row.verdict, Verdict::Same { .. })
            && row
                .left_bytes
                .is_some_and(|bytes| bytes <= budget.max_bytes_per_file);
        let out_of_budget =
            summary.confirmed_same + summary.confirmed_different >= budget.max_files;
        if !promotable || out_of_budget {
            // A row that was already a difference has an answer and needs no
            // verification. A row that was "same" and did not get verified -
            // over budget, over the per-file ceiling, or unreadable - has no
            // answer beyond the metadata one, and the summary must say so
            // rather than let it read as settled.
            if matches!(row.verdict, Verdict::Same { .. }) {
                summary.unverified += 1;
            }
            rows.push(row);
            continue;
        }
        let left_path = left_root.join(&row.path);
        let right_path = right_root.join(&row.path);
        match (
            digest_of(
                engine, left_scope, principal, authorizer, &left_path, budget,
            ),
            digest_of(
                engine,
                right_scope,
                principal,
                authorizer,
                &right_path,
                budget,
            ),
        ) {
            (Some(left), Some(right)) => {
                summary.bytes_read += left.bytes_read + right.bytes_read;
                if left.digest == right.digest {
                    summary.confirmed_same += 1;
                    let mut row = row;
                    row.verdict = Verdict::Same {
                        evidence: diskgraph_core::Evidence::Content,
                    };
                    rows.push(row);
                } else {
                    summary.confirmed_different += 1;
                    let mut row = row;
                    row.verdict = Verdict::Different {
                        reason: diskgraph_core::DifferentReason::Content,
                    };
                    rows.push(row);
                }
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
    digest: String,
    bytes_read: u64,
}

fn digest_of(
    engine: &Engine,
    scope: &ScopeId,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    path: &PathBuf,
    budget: VerifyBudget,
) -> Option<OneDigest> {
    let request = InspectionRequest {
        scope_id: scope,
        principal,
        path,
        offset: 0,
        max_bytes: budget.max_bytes_per_file,
        cancel: None,
        chunk_bytes: 1 << 20,
    };
    let outcome: DigestOutcome = engine
        .digest_bounded(&request, &ConservativeProbe, authorizer)
        .ok()?;
    // A digest that stopped is not a digest: the bytes were not all read, so
    // the hash says nothing about the file.
    if !outcome.confirmed() || outcome.digest_hex.is_empty() {
        return None;
    }
    Some(OneDigest {
        digest: outcome.digest_hex,
        bytes_read: outcome.bytes_digested,
    })
}
