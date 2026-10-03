//! specialist_verdict：既有文件操作职责的原生 Rust 实现。
use crate::specialist::run_outcome::RunOutcome;

/// 结合完整输出和预期标记保守判断专家执行结果。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::SpecialistVerdict`，保留既有语义。
/// What an external run proved. Interpretation is conservative: only a clean
/// exit with the expected marker in fully-seen output counts as done.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpecialistVerdict {
    /// The expected effect is visible in the tool's own report.
    Confirmed,
    /// The outcome cannot be proven: the operation parks for reconciliation
    /// instead of being reported as a success nobody verified (OP-08).
    NeedsAttention { reason: String },
}

/// 保守核对专家执行输出标记。
/// 参数：outcome 为退出和完整性结果；expected_marker 为预期效果标记。
/// 返回：Confirmed 或带原因的 NeedsAttention。
/// Judges a specialist run against the marker its own output must contain to
/// prove the effect happened. Exit codes alone prove nothing: a tool may exit
/// 0 after doing nothing, and output past the cap may hold the only evidence.
pub fn verify_specialist_result(outcome: &RunOutcome, expected_marker: &str) -> SpecialistVerdict {
    if outcome.timed_out {
        return SpecialistVerdict::NeedsAttention {
            reason: "the specialist command timed out; its effect is unknown".into(),
        };
    }
    if outcome.truncated {
        return SpecialistVerdict::NeedsAttention {
            reason: "the specialist command's output was truncated; its effect is unproven".into(),
        };
    }
    if outcome.exit_code != 0 {
        return SpecialistVerdict::NeedsAttention {
            reason: format!(
                "the specialist command exited with {}; its effect is unproven",
                outcome.exit_code
            ),
        };
    }
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    if stdout.contains(expected_marker) || stderr.contains(expected_marker) {
        SpecialistVerdict::Confirmed
    } else {
        SpecialistVerdict::NeedsAttention {
            reason: format!(
                "the specialist command's output does not mention {expected_marker:?}; its effect is unproven"
            ),
        }
    }
}
