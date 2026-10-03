//! adapter_probe：既有文件操作职责的原生 Rust 实现。
use crate::specialist::adapter_capability::AdapterCapability;
use crate::specialist::adapter_status::AdapterStatus;
use crate::specialist::command_runner::CommandRunner;
use crate::specialist::command_spec::CommandSpec;
use crate::specialist::sandboxed_runner::default_env;
use std::path::Path;

/// 通过现有执行器探测允许工具版本。
/// 参数：capability 指定版本要求；program 为固定路径；runner 为执行器。
/// 返回：Available 或明确 Unavailable 原因。
/// Probes a resolved program through the sandboxed runner: it must exist,
/// answer `--version` with exit 0, and report a version at or above the
/// capability's minimum.
pub fn probe(
    capability: &AdapterCapability,
    program: &Path,
    runner: &dyn CommandRunner,
) -> AdapterStatus {
    let spec = CommandSpec {
        program: program.to_path_buf(),
        args: vec!["--version".into()],
        cwd: None,
        env: default_env(),
        timeout_ms: 10_000,
        max_output_bytes: 4_096,
        retries: 0,
    };
    let outcome = match runner.run(&spec) {
        Ok(outcome) => outcome,
        Err(error) => {
            return AdapterStatus::Unavailable {
                reason: format!("{} could not be run: {error}", capability.program),
            };
        }
    };
    if outcome.timed_out {
        return AdapterStatus::Unavailable {
            reason: format!("{} --version timed out", capability.program),
        };
    }
    if outcome.exit_code != 0 {
        return AdapterStatus::Unavailable {
            reason: format!(
                "{} --version exited with {}",
                capability.program, outcome.exit_code
            ),
        };
    }
    let version = String::from_utf8_lossy(&outcome.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    if version.is_empty() {
        return AdapterStatus::Unavailable {
            reason: format!("{} printed no version line", capability.program),
        };
    }
    let found = version
        .split_whitespace()
        .find(|token| token.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or("");
    if version_at_least(found, capability.min_version) {
        AdapterStatus::Available { version }
    } else {
        AdapterStatus::Unavailable {
            reason: format!(
                "{} reports {found}, below the required {}",
                capability.program, capability.min_version
            ),
        }
    }
}

/// 比较工具输出中的数字版本。
/// 参数：actual 与 minimum 为观察和最低版本。
/// 返回：原有数字分量比较结果。
/// Dotted numeric comparison, tolerant of suffixes: "1.75" >= "1.70.0".
pub(super) fn version_at_least(found: &str, minimum: &str) -> bool {
    fn numbers(version: &str) -> Vec<u64> {
        version
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse().ok())
            .collect()
    }
    let found = numbers(found);
    let minimum = numbers(minimum);
    for index in 0..minimum.len().max(found.len()) {
        let left = found.get(index).copied().unwrap_or(0);
        let right = minimum.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    true
}
