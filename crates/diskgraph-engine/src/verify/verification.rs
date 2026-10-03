//! 逐文件对提升比较证据并记录失败成本。

use super::one_digest::OneDigest;
use super::{VerifyBudget, VerifySummary};
use crate::content::{ConservativeProbe, DigestOutcome, InspectionRequest};
use crate::{ComparisonReport, Engine, EngineError, VerifyLimits};
use diskgraph_core::{Authorizer, PrincipalId, ResourceLocator, ScopeId, Verdict};
use std::path::PathBuf;

/// 仅对元数据比较的同类普通文件尝试内容升级。
/// 参数：engine/report、双侧 scope、请求身份与 budget为文件对及单文件预算。
/// 返回：更新报告与真实成本统计或定位失败；不可读取者保留未核验。
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
/// 仅对元数据比较的同类普通文件尝试内容升级。
/// 参数：engine/report、双侧 scope、请求身份与 budget/limits 为累计字节/期限/取消约束。
/// 返回：更新报告与真实成本统计或定位失败；不可读取者保留未核验。
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
    let deadline = std::time::Instant::now()
        .checked_add(std::time::Duration::from_millis(limits.max_duration_ms))
        .ok_or(diskgraph_core::BusinessError::InvalidArgument)?;
    verify_same_rows_with_limits_until(
        engine,
        report,
        left_scope,
        right_scope,
        principal,
        authorizer,
        budget,
        limits,
        deadline,
    )
}

/// 内容核验沿用比较请求已经建立的绝对期限。
/// 参数：engine/report、双侧 scope、身份与 budget 沿用旧契约，deadline 为首次准备前期限。
/// 返回：报告和实际双侧读取成本；到期标记 deadline，不重新获得默认三十秒。
#[allow(clippy::too_many_arguments)] // 双侧内容上下文与请求期限独立。
pub fn verify_same_rows_until(
    engine: &Engine,
    report: ComparisonReport,
    left_scope: &ScopeId,
    right_scope: &ScopeId,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    budget: VerifyBudget,
    deadline: std::time::Instant,
) -> Result<(ComparisonReport, VerifySummary), EngineError> {
    verify_same_rows_with_limits_until(
        engine,
        report,
        left_scope,
        right_scope,
        principal,
        authorizer,
        budget,
        &VerifyLimits::default(),
        deadline,
    )
}

/// 核验自有文件/累计字节上限与查询共同期限取更早者。
/// 参数：旧 engine/report/scope/identity/budget/limits 上下文，以及最外层 deadline。
/// 返回：报告和实际失败成本；内容权限拒绝不确认摘要，适配器负责报告元数据末检。
#[allow(clippy::too_many_arguments)] // 保留既有独立限制与双侧请求上下文。
pub fn verify_same_rows_with_limits_until(
    engine: &Engine,
    report: ComparisonReport,
    left_scope: &ScopeId,
    right_scope: &ScopeId,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    budget: VerifyBudget,
    limits: &VerifyLimits,
    deadline: std::time::Instant,
) -> Result<(ComparisonReport, VerifySummary), EngineError> {
    let own_deadline = std::time::Instant::now()
        .checked_add(std::time::Duration::from_millis(limits.max_duration_ms))
        .ok_or(diskgraph_core::BusinessError::InvalidArgument)?;
    let deadline = deadline.min(own_deadline);
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
    if std::time::Instant::now() >= deadline {
        report.truncated = Some("deadline");
    }
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
