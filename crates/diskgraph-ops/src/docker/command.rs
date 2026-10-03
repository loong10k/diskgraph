//! command：既有文件操作职责的原生 Rust 实现。
use crate::OpsError;
use crate::specialist::CommandRunner;
use crate::specialist::CommandSpec;
use crate::specialist::RunOutcome;
use crate::specialist::SandboxedRunner;
use std::path::Path;

/// 构造 Docker 结构化命令。
/// 参数：docker 为固定程序路径；args 为逐项参数。
/// 返回：原有超时、逐流上限与环境的 CommandSpec。
pub(super) fn spec(docker: &Path, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: docker.to_path_buf(),
        args: args.iter().map(|argument| argument.to_string()).collect(),
        cwd: None,
        env: SandboxedRunner::default_env(),
        timeout_ms: 30_000,
        max_output_bytes: 4 << 20,
        retries: 0,
    }
}

/// 通过调用者提供的执行器运行 Docker 结构化命令。
/// 参数：runner 为执行器；docker 为固定程序路径；args 为逐项 argv。
/// 返回：执行器的 RunOutcome；超时转为 Stale，其他执行错误原样传播；平台能力由 runner 决定。
pub(super) fn run(
    runner: &dyn CommandRunner,
    docker: &Path,
    args: &[&str],
) -> Result<RunOutcome, OpsError> {
    let outcome = runner.run(&spec(docker, args))?;
    if outcome.timed_out {
        return Err(OpsError::Stale(format!(
            "docker {} timed out",
            args.join(" ")
        )));
    }
    Ok(outcome)
}
