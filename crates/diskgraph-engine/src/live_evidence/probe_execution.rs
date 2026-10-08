use super::probe_budget::ProbeBudget;
use super::probe_failure::ProbeFailure;
use super::probe_output::ProbeOutput;
#[cfg(windows)]
use super::probe_windows::WindowsProbeChild as PlatformChild;
#[cfg(unix)]
use super::unix_probe_child::UnixProbeChild as PlatformChild;
use std::process::Command;
use std::time::Duration;

/// 执行受信结构化命令，两管道与后续命令共享同一资源边界。
/// 参数：command 为清空环境的命令，budget 为整次采样共享预算。
/// 返回：仅在进程退出、双 EOF 与末段预算均有效时返回完整输出，否则清理并锁存失败。
pub(super) fn run_probe(
    command: &mut Command,
    budget: &mut ProbeBudget,
) -> Result<ProbeOutput, ProbeFailure> {
    let result = execute(command, budget);
    if let Err(error) = &result {
        // 当前调用保留清理诊断；共享预算仍锁存最早中止原因，后续命令不能重试。
        budget.fail(error.clone());
    }
    result
}

/// 清空继承环境，仅保留定位受信工具与 Windows 系统运行库所需键。
/// 参数：command 为本次采样独占的结构化命令。
/// 返回：无；后端只使用此处明确设置的环境，不继承 Git/loader 配置。
pub(super) fn configure_probe_env(command: &mut Command) {
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
}

#[cfg(unix)]
fn execute(command: &mut Command, budget: &mut ProbeBudget) -> Result<ProbeOutput, ProbeFailure> {
    budget.check()?;
    let mut child = PlatformChild::spawn(command, budget)?;
    let result = collect_output(&mut child, budget);
    let cleanup = child.cleanup();
    match result {
        Ok(output) => output.finish(budget, cleanup),
        Err(primary) => Err(primary.with_cleanup(cleanup)),
    }
}

#[cfg(windows)]
#[path = "windows_probe_execution.rs"]
mod windows_probe_execution;

#[cfg(windows)]
fn execute(command: &mut Command, budget: &mut ProbeBudget) -> Result<ProbeOutput, ProbeFailure> {
    windows_probe_execution::execute(command, budget)
}

#[cfg(any(unix, windows))]
fn collect_output(
    child: &mut PlatformChild,
    budget: &mut ProbeBudget,
) -> Result<ProbeOutput, ProbeFailure> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        budget.check()?;
        let finished = child.poll()?;
        if finished && child.exit_code().is_none_or(|code| code < 0) {
            // Unix 信号死亡不是工具正常非零退出；Windows 高位状态保守拒绝。
            // 不允许 HEAD/upstream 将异常终止降级为缺少引用。
            return Err(ProbeFailure::AbnormalExit(child.exit_code()));
        }
        let mut progress = false;
        // 每轮每流最多 4 KiB。Windows 两项 pending 可额外读取固定 8 KiB；
        // 所有超额度数据只用于拒绝，不追加，不将额度耗尽当作 EOF。
        if let Some(bytes) = child.read_stdout()? {
            budget.consume(bytes.len())?;
            stdout.extend_from_slice(bytes);
            progress = true;
        }
        budget.check()?;
        if let Some(bytes) = child.read_stderr()? {
            budget.consume(bytes.len())?;
            stderr.extend_from_slice(bytes);
            progress = true;
        }
        budget.check()?;
        if finished && child.stdout_eof() && child.stderr_eof() {
            let exit_code = child.exit_code();
            budget.check()?;
            return Ok(ProbeOutput {
                stdout,
                stderr,
                exit_code,
            });
        }
        if !progress {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn execute(_command: &mut Command, _budget: &mut ProbeBudget) -> Result<ProbeOutput, ProbeFailure> {
    Err(ProbeFailure::Unsupported(
        "no native probe execution boundary",
    ))
}
