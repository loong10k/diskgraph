//! 按精确路径采样可见占用并保留覆盖诊断。

use super::probe_budget::ProbeBudget;
use super::probe_execution::{configure_probe_env, run_probe};
use super::sampling_clock::now_ms;
use super::{ProbeLimits, UsageCoverage, UsageSample};
use std::path::Path;
use std::process::Command;

/// 按精确路径采样可见占用并保留覆盖诊断。
/// 参数：lsof 为受信程序路径，paths 为精确采样路径。
/// 返回：使用样本；无法运行/观察时不可当作无人占用。
/// 仅保留可见正向观察；未核验权限范围/PID 启动身份时返回 partial。
/// 空请求没有观察对象，保留兼容空样本，不能将其套用于任何文件。
/// 默认整次采样为 15 秒、两管道累计 1 MiB，安全回收可能超过协作期限。
pub fn sample_process_usage(lsof: &Path, paths: &[&Path]) -> UsageSample {
    sample_process_usage_bounded(lsof, paths, &ProbeLimits::default())
}

/// 使用调用方的整次期限、累计输出与取消配置采样可见句柄。
/// 参数：lsof 为受信程序，paths 为精确路径，limits 为整次采样配置。
/// 返回：完整执行后解释出的 partial 样本，或带资源/平台诊断的不可观察样本。
pub fn sample_process_usage_bounded(
    lsof: &Path,
    paths: &[&Path],
    limits: &ProbeLimits,
) -> UsageSample {
    let sampled_at_unix_ms = now_ms();
    let mut budget = match ProbeBudget::new(limits) {
        Ok(budget) => budget,
        Err(error) => return unobservable(sampled_at_unix_ms, error.to_string()),
    };
    sample_process_usage_using_budget(lsof, paths, &mut budget)
}

/// 借用任务原预算观察下一组路径，不能补充额度或重新起算期限。
/// 参数：lsof 为受信程序，paths 为精确路径，budget 为共享执行预算。
/// 返回：原覆盖语义的样本；执行、格式和终态失败均不可观察。
pub(super) fn sample_process_usage_using_budget(
    lsof: &Path,
    paths: &[&Path],
    budget: &mut ProbeBudget,
) -> UsageSample {
    let sampled_at_unix_ms = now_ms();
    if let Err(error) = budget.check() {
        return unobservable(sampled_at_unix_ms, error.to_string());
    }
    if paths.is_empty() {
        return UsageSample {
            sampled_at_unix_ms,
            coverage: UsageCoverage::Full,
            holders: Vec::new(),
        };
    }
    let mut command = Command::new(lsof);
    // -w: suppress warnings; -F pcn0: machine-readable pid/command/name
    // records, NUL-terminated; `--`: end of options, so a path that begins
    // with `-` is still a path.
    command.args(["-w", "-F", "pcn0", "--"]);
    command.args(paths);
    configure_probe_env(&mut command);
    let output = match run_probe(&mut command, budget) {
        Ok(output) => output,
        Err(error) => return unobservable(sampled_at_unix_ms, error.to_string()),
    };
    let sample = super::process_output::interpret_process_output(
        &output.stdout,
        output.exit_code,
        paths,
        sampled_at_unix_ms,
    );
    if let Err(error) = budget.check() {
        return unobservable(sampled_at_unix_ms, error.to_string());
    }
    sample
}

/// 构造没有成功占用断言的失败样本。参数：sampled_at_unix_ms 为观察时间，reason 为完整诊断。返回：不可观察覆盖。
pub(super) fn unobservable(sampled_at_unix_ms: u64, reason: String) -> UsageSample {
    UsageSample {
        sampled_at_unix_ms,
        coverage: UsageCoverage::Unobservable { reason },
        holders: Vec::new(),
    }
}
