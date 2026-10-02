//! 用既有本地 Git 命令采样项目状态。

use super::GitSample;
use super::sampling_clock::now_ms;
use std::path::Path;
use std::process::Command;

/// 用既有本地 Git 命令采样项目状态。
/// 参数：git 为受信程序路径，project 为项目目录。
/// 返回：本地 Git 样本或命令错误；兼容实现的配置隔离与执行预算尚未完成。
/// Samples one repository with the local `git` binary. Everything runs with
/// the project as cwd and a minimal environment. It does not explicitly invoke
/// fetch, but repository configuration can execute external programs. This
/// trusted compatibility path cannot certify offline/read-only execution and
/// still lacks a complete subprocess budget/configuration isolation boundary.
pub fn sample_git(git: &Path, project: &Path) -> Result<GitSample, String> {
    let run = |args: &[&str]| -> Result<String, String> {
        let output = Command::new(git)
            .args(args)
            .current_dir(project)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|error| format!("git could not run: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("git failed")
                .to_owned());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };

    let head = run(&["rev-parse", "HEAD"]).ok();
    if head.is_none() {
        // Distinguish "not a repository" from other failures.
        run(&["rev-parse", "--is-inside-work-tree"])
            .map_err(|reason| format!("not a usable git repository: {reason}"))?;
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
    let dirty_count = run(&["status", "--porcelain"])?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u64;
    let stash_count = run(&["stash", "list"])?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u64;
    let mut notes = Vec::new();
    let mut ahead = None;
    let mut behind = None;
    match run(&[
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "@{upstream}",
    ]) {
        Ok(upstream) => {
            let upstream = upstream.trim().to_owned();
            ahead = run(&["rev-list", "--count", &format!("{upstream}..HEAD")])?
                .trim()
                .parse()
                .ok();
            behind = run(&["rev-list", "--count", &format!("HEAD..{upstream}")])?
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
