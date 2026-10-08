//! 用既有本地 Git 命令采样项目状态。

use super::git_metadata_budget::GitMetadataBudget;
use super::git_output::{status_count, successful};
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
/// the trusted Git package's shell. Objects are bounded private flat copies;
/// source alternates/promisor stores are unsupported. This is not an atomic
/// repository snapshot, and larger object stores may exceed the shared quota.
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
/// loose/pack 副本及末段重读共用元数据额度；安全名单排序和对象复制增加成本。
/// 保留初始对象 Vec，终检暂有新旧两份数据，不承诺全部 Git RSS 上限。
/// 工具在首次命令前解析成固定绝对路径，仅搜索绝对 PATH 项；Windows 要求原生 .exe。
pub fn sample_git_bounded(
    git: &Path,
    project: &Path,
    limits: &ProbeLimits,
) -> Result<GitSample, String> {
    sample_git_with_resources(
        git,
        project,
        limits,
        GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
}

/// 使用同一真实采样流程注入私有资源额度，不改变公开默认能力。
/// 参数：git/project/limits 为原公开请求，metadata_budget 为全程共享输入额度；
/// allocation_quota/min_free 为私有对象报告分配及卷余量门禁。
/// 返回：仅完整观察及显式清理成功时返回样本，失败不重试或重置额度。
pub(super) fn sample_git_with_resources(
    git: &Path,
    project: &Path,
    limits: &ProbeLimits,
    metadata_budget: GitMetadataBudget,
    allocation_quota: u64,
    min_free: u64,
) -> Result<GitSample, String> {
    let mut budget = ProbeBudget::new(limits).map_err(|error| error.to_string())?;
    sample_git_using_budget(
        git,
        project,
        &mut budget,
        metadata_budget,
        allocation_quota,
        min_free,
    )
    .map(|(sample, _remaining)| sample)
}

/// 借用任务预算并移动元数据额度，多个目标不能重新获得期限或字节。
/// 参数：git/project 为受信工具及目录，budget 为共享执行预算，metadata_budget 为原剩余额度；
/// allocation_quota/min_free 为当前私有视图分配与卷余量门禁。
/// 返回：完整样本及清理后真实剩余输入额度，或包含清理诊断的失败。
pub(super) fn sample_git_using_budget(
    git: &Path,
    project: &Path,
    budget: &mut ProbeBudget,
    metadata_budget: GitMetadataBudget,
    allocation_quota: u64,
    min_free: u64,
) -> Result<(GitSample, GitMetadataBudget), String> {
    let mut view = GitView::prepare(
        git,
        project,
        budget,
        metadata_budget,
        allocation_quota,
        min_free,
    )?;
    let result = observe(&mut view, budget);
    view.complete_with_metadata(result, budget)
}

/// 复用同一视图完成真实采样。参数：view 为已准备视图，budget 为原执行预算；返回：经终检的样本。
pub(super) fn observe(view: &mut GitView, budget: &mut ProbeBudget) -> Result<GitSample, String> {
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
            let (observed_ahead, observed_behind) =
                super::git_divergence::count(head, &upstream, &mut |args| run(args, budget))?;
            ahead = Some(observed_ahead);
            behind = Some(observed_behind);
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
    git_references::verify_head(&mut |args| run(args, budget), &observed_head)?;
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
