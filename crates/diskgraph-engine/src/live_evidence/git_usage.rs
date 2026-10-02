//! 用既有本地 Git 命令采样项目状态。

use super::git_output::{commit_count, status_count, successful};
use super::probe_budget::ProbeBudget;
use super::probe_execution::{configure_probe_env, run_probe};
use super::probe_output::ProbeOutput;
use super::sampling_clock::now_ms;
use super::{GitSample, ProbeLimits};
use super::{git_references, git_stash};
use std::path::Path;
use std::process::Command;

/// 用既有本地 Git 命令采样项目状态。
/// 参数：git 为受信程序路径，project 为项目目录。
/// 返回：本地 Git 样本或错误；默认整次 15 秒/管道及 stash 日志累计 1 MiB。
/// Samples one repository with the local `git` binary. Everything runs with
/// the project as cwd and a minimal environment. It does not explicitly invoke
/// fetch and disables Git's lazy fetch and optional lock-based updates, but
/// repository configuration can still execute external programs. This
/// trusted compatibility path cannot certify offline/read-only execution and
/// configuration isolation remains unverified. Resource failures are errors.
/// Windows 工具/绝对目录及 Unix 宿主条件见 sample_git_bounded 与 ProbeLimits。
pub fn sample_git(git: &Path, project: &Path) -> Result<GitSample, String> {
    sample_git_bounded(git, project, &ProbeLimits::default())
}

/// Git 多个子命令共用一次采样期限、累计输出与取消配置。
/// 参数：git 为受信工具，project 为项目目录，limits 为整次资源配置。
/// 返回：本地样本或明确错误；资源失败不能表示无 HEAD/upstream，配置隔离仍待验收。
/// 要求 Git 2.46+ 的引用存在性接口；不支持时返回错误，不回退为猜测缺引用。
/// stash 存在时仅支持可核验的 files 引用后端，不把跳过的日志记录当完整计数。
/// 固定禁用 pager、懒取与可选锁写入；这不隔离 filter/fsmonitor 或 shared index 刷新。
/// Windows 要求 project 为绝对目录，工具为受信 .exe 或显式 PATH 中的 .exe。
pub fn sample_git_bounded(
    git: &Path,
    project: &Path,
    limits: &ProbeLimits,
) -> Result<GitSample, String> {
    let mut budget = ProbeBudget::new(limits).map_err(|error| error.to_string())?;
    let mut run = |args: &[&str], budget: &mut ProbeBudget| -> Result<ProbeOutput, String> {
        let mut command = Command::new(git);
        // 所有 HEAD/status/stash/upstream 命令采用同一策略，缺对象不能隐式下载。
        // optional locks 不是完整只读边界，配置回调与 shared index 仍待私有视图隔离。
        command
            .args(["--no-pager", "--no-lazy-fetch", "--no-optional-locks"])
            .args(args)
            .current_dir(project);
        configure_probe_env(&mut command);
        command.env("GIT_TERMINAL_PROMPT", "0");
        run_probe(&mut command, budget).map_err(|error| error.to_string())
    };

    let observed_head = git_references::head(&mut |args| run(args, &mut budget))?;
    let dirty_count = status_count(&successful(run(
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        &mut budget,
    )?)?)?;
    let stash_count = if git_references::exists(&mut |args| run(args, &mut budget), "refs/stash")? {
        git_stash::count(&mut run, &mut budget)?
    } else {
        0
    };
    let mut notes = Vec::new();
    let mut ahead = None;
    let mut behind = None;
    if let (Some(head), Some(branch)) = (&observed_head.0, &observed_head.1) {
        let (upstream, mapped) =
            git_references::upstream(&mut |args| run(args, &mut budget), branch, head)?;
        if let Some(upstream) = upstream {
            // 只使用完整 OID 范围，引用名不成为 revision 表达式或额外选项。
            ahead = Some(commit_count(&successful(run(
                &["rev-list", "--count", &format!("{upstream}..{head}")],
                &mut budget,
            )?)?)?);
            behind = Some(commit_count(&successful(run(
                &["rev-list", "--count", &format!("{head}..{upstream}")],
                &mut budget,
            )?)?)?);
        } else if mapped {
            notes.push("configured upstream reference is unavailable locally; whether commits are pushed is unknown and is never reported as pushed".into());
        } else {
            notes.push("no locally resolvable upstream is configured; whether commits are pushed is unknown and is never reported as pushed".into());
        }
    } else if observed_head.0.is_none() {
        notes.push(
            "the repository has no commits yet; actual working-tree changes are included".into(),
        );
    } else {
        notes.push("detached HEAD has no branch upstream; whether commits are pushed is unknown and is never reported as pushed".into());
    }
    if git_references::head(&mut |args| run(args, &mut budget))? != observed_head {
        return Err("HEAD changed during Git sampling".into());
    }
    budget.check().map_err(|error| error.to_string())?;
    Ok(GitSample {
        head: observed_head.0,
        dirty_count,
        stash_count,
        ahead_of_upstream: ahead,
        behind_upstream: behind,
        notes,
        sampled_at_unix_ms: now_ms(),
    })
}
