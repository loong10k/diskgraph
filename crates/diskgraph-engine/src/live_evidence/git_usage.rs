//! 用既有本地 Git 命令采样项目状态。

use super::probe_budget::ProbeBudget;
use super::probe_execution::{configure_probe_env, run_probe};
use super::probe_output::ProbeOutput;
use super::sampling_clock::now_ms;
use super::{GitSample, ProbeLimits};
use std::path::Path;
use std::process::Command;

/// 用既有本地 Git 命令采样项目状态。
/// 参数：git 为受信程序路径，project 为项目目录。
/// 返回：本地 Git 样本或命令错误；默认整次 15 秒/两管道累计 1 MiB。
/// Samples one repository with the local `git` binary. Everything runs with
/// the project as cwd and a minimal environment. It does not explicitly invoke
/// fetch, but repository configuration can execute external programs. This
/// trusted compatibility path cannot certify offline/read-only execution and
/// configuration isolation remains unverified. Resource failures are errors.
/// Windows 工具/绝对目录及 Unix 宿主条件见 sample_git_bounded 与 ProbeLimits。
pub fn sample_git(git: &Path, project: &Path) -> Result<GitSample, String> {
    sample_git_bounded(git, project, &ProbeLimits::default())
}

/// Git 多个子命令共用一次采样期限、累计输出与取消配置。
/// 参数：git 为受信工具，project 为项目目录，limits 为整次资源配置。
/// 返回：本地样本或明确错误；资源失败不能表示无 HEAD/upstream，配置隔离仍待验收。
/// Windows 要求 project 为绝对目录，工具为受信 .exe 或显式 PATH 中的 .exe。
pub fn sample_git_bounded(
    git: &Path,
    project: &Path,
    limits: &ProbeLimits,
) -> Result<GitSample, String> {
    let mut budget = ProbeBudget::new(limits).map_err(|error| error.to_string())?;
    let mut run = |args: &[&str]| -> Result<ProbeOutput, String> {
        let mut command = Command::new(git);
        command.args(args).current_dir(project);
        configure_probe_env(&mut command);
        command.env("GIT_TERMINAL_PROMPT", "0");
        run_probe(&mut command, &mut budget).map_err(|error| error.to_string())
    };

    // 仅工具自身的非零退出保留旧缺引用语义，执行边界错误先用 ? 传播。
    let head = successful_text(run(&["rev-parse", "HEAD"])?).ok();
    if head.is_none() {
        // Distinguish "not a repository" from other failures.
        successful_text(run(&["rev-parse", "--is-inside-work-tree"])?)
            .map_err(|reason| format!("not a usable git repository: {reason}"))?;
        budget.check().map_err(|error| error.to_string())?;
        return Ok(GitSample {
            head: None,
            dirty_count: 0,
            stash_count: 0,
            ahead_of_upstream: None,
            behind_upstream: None,
            notes: vec!["the repository has no commits yet".into()],
            sampled_at_unix_ms: now_ms(),
        });
    }
    let head = head.map(|text| text.trim().to_owned());
    let dirty_count = successful_text(run(&["status", "--porcelain"])?)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u64;
    let stash_count = successful_text(run(&["stash", "list"])?)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u64;
    let mut notes = Vec::new();
    let mut ahead = None;
    let mut behind = None;
    match successful_text(run(&[
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "@{upstream}",
    ])?) {
        Ok(upstream) => {
            let upstream = upstream.trim().to_owned();
            ahead = successful_text(run(&["rev-list", "--count", &format!("{upstream}..HEAD")])?)?
                .trim()
                .parse()
                .ok();
            behind = successful_text(run(&["rev-list", "--count", &format!("HEAD..{upstream}")])?)?
                .trim()
                .parse()
                .ok();
        }
        Err(_) => {
            notes.push(
                "no upstream is configured: whether local commits are pushed is unknown, \
                 and is never reported as pushed"
                    .into(),
            );
        }
    }
    budget.check().map_err(|error| error.to_string())?;
    Ok(GitSample {
        head,
        dirty_count,
        stash_count,
        ahead_of_upstream: ahead,
        behind_upstream: behind,
        notes,
        sampled_at_unix_ms: now_ms(),
    })
}

fn successful_text(output: ProbeOutput) -> Result<String, String> {
    if output.exit_code != Some(0) {
        return Err(String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or("git failed")
            .to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
