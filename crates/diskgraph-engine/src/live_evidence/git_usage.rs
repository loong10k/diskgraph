//! 用既有本地 Git 命令采样项目状态。

use super::git_output::{commit_count, status_count, successful};
use super::git_view::GitView;
use super::probe_budget::ProbeBudget;
use super::probe_output::ProbeOutput;
use super::sampling_clock::now_ms;
use super::{GitSample, ProbeLimits};
use super::{git_references, git_stash};
use std::path::Path;

/// 用既有本地 Git 命令采样项目状态。
/// 参数：git 为受信程序路径，project 为项目目录。
/// 返回：本地 Git 样本或错误；默认整次 15 秒/管道及 stash 日志累计 1 MiB。
/// Samples a supported repository through private configuration, index and
/// reference copies. External filter/fsmonitor commands are never copied.
/// Unsupported semantics and resource failures are errors. A fixed printer
/// discovers the host config path before bounded native capture; it requires
/// the trusted Git package's shell. A borrowed recursive object store retains
/// separate input/access limits; this is not an atomic repository snapshot.
/// Windows 工具/绝对目录及 Unix 宿主条件见 sample_git_bounded 与 ProbeLimits。
pub fn sample_git(git: &Path, project: &Path) -> Result<GitSample, String> {
    sample_git_bounded(git, project, &ProbeLimits::default())
}

/// Git 多个子命令共用一次采样期限、累计输出与取消配置。
/// 参数：git 为受信工具，project 为项目目录，limits 为整次资源配置。
/// 返回：私有视图的本地样本或明确错误；资源失败不能表示无 HEAD/upstream。
/// 要求 Git 2.46+ 的引用存在性接口；不支持时返回错误，不回退为猜测缺引用。
/// stash 存在时仅支持可核验的 files 引用后端，不把跳过的日志记录当完整计数。
/// 固定禁用 pager、懒取与可选锁写入；特殊 filter/index/配置无法保真时明确拒绝。
/// 准备与复核同期限/取消；元数据另有两轮累计 64 MiB/32k 条目额度，非严格 RSS 上限。
/// 私有对象另限原生报告分配 128 MiB，卷余量至少 64 MiB；时点检查不预留空间。
/// 工具在首次命令前解析成固定绝对路径，仅搜索绝对 PATH 项；Windows 要求原生 .exe。
pub fn sample_git_bounded(
    git: &Path,
    project: &Path,
    limits: &ProbeLimits,
) -> Result<GitSample, String> {
    let mut budget = ProbeBudget::new(limits).map_err(|error| error.to_string())?;
    let mut view = GitView::prepare(git, project, &mut budget)?;
    let result = observe(&mut view, &mut budget);
    view.complete(result).and_then(|sample| {
        // 安全删除可以跨过协作期限；清理后的取消或超期仍不得发布成功样本。
        budget.check().map_err(|error| error.to_string())?;
        Ok(sample)
    })
}

fn observe(view: &mut GitView, budget: &mut ProbeBudget) -> Result<GitSample, String> {
    let mut run = |args: &[&str], budget: &mut ProbeBudget| -> Result<ProbeOutput, String> {
        view.run(args, budget)
    };

    let observed_head = git_references::head(&mut |args| run(args, budget))?;
    let dirty_count = status_count(&successful(run(
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        budget,
    )?)?)?;
    let stash_count = if git_references::exists(&mut |args| run(args, budget), "refs/stash")? {
        git_stash::count(&mut run, budget)?
    } else {
        0
    };
    let mut notes = Vec::new();
    let mut ahead = None;
    let mut behind = None;
    if let (Some(head), Some(branch)) = (&observed_head.0, &observed_head.1) {
        let (upstream, mapped) =
            git_references::upstream(&mut |args| run(args, budget), branch, head)?;
        if let Some(upstream) = upstream {
            // 只使用完整 OID 范围，引用名不成为 revision 表达式或额外选项。
            ahead = Some(commit_count(&successful(run(
                &["rev-list", "--count", &format!("{upstream}..{head}")],
                budget,
            )?)?)?);
            behind = Some(commit_count(&successful(run(
                &["rev-list", "--count", &format!("{head}..{upstream}")],
                budget,
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
    if git_references::head(&mut |args| run(args, budget))? != observed_head {
        return Err("HEAD changed during Git sampling".into());
    }
    view.verify(budget)?;
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
