//! 真实仓库夹具：Git 的 stash list 成功退出仍可能静默跳过损坏 reflog 条目。

#[cfg(windows)]
use super::native_evidence_test_session::sample_git_bounded;
#[cfg(not(windows))]
use super::sample_git_bounded;
use super::{GitSample, ProbeLimits};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

// 此夹具验证 stash 语义；显式有限测试期限容纳共享 CI 调度，生产默认仍为 15 秒。
// 输出、取消和私有输入/分配额度沿用真实默认值，不重试失败采样。
fn sample_semantics(git: &Path, project: &Path) -> Result<GitSample, String> {
    let limits = ProbeLimits {
        timeout: Duration::from_secs(60),
        ..ProbeLimits::default()
    };
    let started = Instant::now();
    sample_git_bounded(git, project, &limits).map_err(|error| {
        format!(
            "{error}; stash semantic fixture elapsed={}ms, timeout={}ms",
            started.elapsed().as_millis(),
            limits.timeout.as_millis()
        )
    })
}

// 负例必须因目标语义被拒绝；超时、取消或资源耗尽均不能替代校验成功。
fn assert_semantic_rejection(project: &Path, expected: &[&str]) {
    let error = sample_semantics(Path::new("git"), project).unwrap_err();
    assert!(
        expected.iter().any(|prefix| error.starts_with(prefix)),
        "mutated stash fixture expected {expected:?}, observed {error}"
    );
}

fn git(project: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(["-c", "user.name=fixture", "-c", "user.email=f@example"])
        .args(args)
        .current_dir(project)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    output.stdout
}

fn two_stashes() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    std::fs::create_dir(&project).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(&project, &["commit", "--allow-empty", "-q", "-m", "base"]);
    for content in [b"first".as_slice(), b"second"] {
        std::fs::write(project.join("tracked"), content).unwrap();
        git(&project, &["add", "tracked"]);
        git(&project, &["stash", "push", "-q"]);
    }
    assert_eq!(
        sample_semantics(Path::new("git"), &project)
            .expect("baseline: two valid stash entries must be observable")
            .stash_count,
        2
    );
    (temp, project)
}

#[test]
fn damaged_older_reflog_record_cannot_be_silently_omitted() {
    let (_temp, project) = two_stashes();
    let log = project.join(".git/logs/refs/stash");
    let mut lines = std::fs::read(&log).unwrap();
    assert_eq!(lines.iter().filter(|byte| **byte == b'\n').count(), 2);
    // 有效宽度但不存在的旧 stash OID，Git list 会静默跳过这一条。
    let first_new = lines[41..81].to_vec();
    let missing = if first_new.iter().all(|byte| *byte == b'1') {
        b'2'
    } else {
        b'1'
    };
    lines[41..81].fill(missing);
    std::fs::write(&log, lines).unwrap();
    assert_semantic_rejection(&project, &["git exit status Some(128);"]);
}

#[test]
fn missing_older_stash_commit_cannot_be_silently_omitted() {
    let (_temp, project) = two_stashes();
    let log = std::fs::read(project.join(".git/logs/refs/stash")).unwrap();
    let oid = std::str::from_utf8(&log[41..81]).unwrap();
    let object = project.join(".git/objects").join(&oid[..2]).join(&oid[2..]);
    assert!(
        object.is_file(),
        "fixture expects the isolated loose stash object"
    );
    std::fs::remove_file(object).unwrap();
    assert_semantic_rejection(&project, &["git exit status Some(128);"]);
}

#[test]
fn drop_and_expire_are_not_mistaken_for_corruption() {
    let (_temp, project) = two_stashes();
    git(&project, &["stash", "drop", "-q", "stash@{1}"]);
    assert_eq!(
        sample_semantics(Path::new("git"), &project)
            .unwrap()
            .stash_count,
        1
    );
    git(
        &project,
        &["reflog", "expire", "--expire=all", "refs/stash"],
    );
    assert_eq!(
        sample_semantics(Path::new("git"), &project)
            .unwrap()
            .stash_count,
        0
    );
}

#[test]
fn malformed_raw_record_is_not_a_valid_stash_count() {
    let (_temp, project) = two_stashes();
    let log = project.join(".git/logs/refs/stash");
    let mut bytes = std::fs::read(&log).unwrap();
    bytes[40] = b'X';
    std::fs::write(log, bytes).unwrap();
    assert_semantic_rejection(&project, &["invalid stash reflog record"]);
}

#[test]
fn valid_stash_ref_without_raw_log_has_zero_listed_stashes() {
    let (_temp, project) = two_stashes();
    std::fs::remove_file(project.join(".git/logs/refs/stash")).unwrap();
    assert_eq!(
        sample_semantics(Path::new("git"), &project)
            .unwrap()
            .stash_count,
        0
    );
}

#[test]
fn reflog_delete_without_rewrite_does_not_require_old_oid_chain() {
    let (_temp, project) = two_stashes();
    git(&project, &["reflog", "delete", "stash@{1}"]);
    assert_eq!(
        sample_semantics(Path::new("git"), &project)
            .unwrap()
            .stash_count,
        1
    );
}

#[test]
fn committer_name_containing_tab_is_a_valid_raw_record() {
    let (_temp, project) = two_stashes();
    let log = project.join(".git/logs/refs/stash");
    let bytes = std::fs::read(&log).unwrap();
    let needle = b"fixture <";
    let pos = bytes
        .windows(needle.len())
        .position(|part| part == needle)
        .unwrap();
    let mut edited = bytes;
    edited.splice(pos..pos + needle.len(), b"fix\tture <".iter().copied());
    std::fs::write(log, edited).unwrap();
    assert_eq!(
        sample_semantics(Path::new("git"), &project)
            .unwrap()
            .stash_count,
        2
    );
}

#[test]
fn message_containing_angle_brackets_and_greater_than_is_not_identity() {
    let (_temp, project) = two_stashes();
    std::fs::write(project.join("tracked"), b"third").unwrap();
    git(&project, &["add", "tracked"]);
    git(
        &project,
        &["stash", "push", "-q", "-m", "compare > value <sample>"],
    );
    assert_eq!(
        sample_semantics(Path::new("git"), &project)
            .unwrap()
            .stash_count,
        3
    );
}

#[test]
fn linked_worktree_uses_gits_shared_reflog_location() {
    let (temp, project) = two_stashes();
    let linked = temp.path().join("linked");
    git(
        &project,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            linked.to_str().unwrap(),
        ],
    );
    assert_eq!(
        sample_semantics(Path::new("git"), &linked)
            .unwrap()
            .stash_count,
        2
    );
}

#[cfg(unix)]
#[test]
fn symlinked_reflog_is_rejected_even_if_contents_are_valid() {
    use super::ProbeLimits;
    use super::git_reflog_file::GitReflogFile;
    use super::probe_budget::ProbeBudget;
    use std::os::unix::fs::symlink;
    let (temp, project) = two_stashes();
    let log = project.canonicalize().unwrap().join(".git/logs/refs/stash");
    let moved = temp.path().join("moved-stash-log");
    std::fs::rename(&log, &moved).unwrap();
    symlink(&moved, &log).unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let error = GitReflogFile::open(&log, &mut budget)
        .err()
        .expect("reject link");
    assert!(error.starts_with("open stash reflog:"), "{error}");
    let link_error = format!(
        "git metadata leaf: {}",
        std::io::Error::from_raw_os_error(libc::ELOOP)
    );
    assert_semantic_rejection(&project, &[&link_error]);
}

#[cfg(unix)]
#[test]
fn fifo_reflog_is_rejected_without_waiting_for_a_writer() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let (_temp, project) = two_stashes();
    let log = project.join(".git/logs/refs/stash");
    std::fs::remove_file(&log).unwrap();
    let native = CString::new(log.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);
    assert_semantic_rejection(&project, &["git metadata is not a regular file"]);
}

#[cfg(unix)]
#[test]
fn symlinked_reflog_parent_is_rejected() {
    use super::ProbeLimits;
    use super::git_reflog_file::GitReflogFile;
    use super::probe_budget::ProbeBudget;
    use std::os::unix::fs::symlink;
    let (temp, project) = two_stashes();
    let refs = project.canonicalize().unwrap().join(".git/logs/refs");
    let moved = temp.path().join("moved-refs");
    std::fs::rename(&refs, &moved).unwrap();
    symlink(&moved, &refs).unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let error = GitReflogFile::open(&refs.join("stash"), &mut budget)
        .err()
        .expect("reject linked parent");
    assert!(error.starts_with("stash path parent:"), "{error}");
    assert_semantic_rejection(
        &project,
        &[
            "unsupported Git metadata directory type",
            "unsupported Git ref link",
        ],
    );
}

#[cfg(unix)]
#[test]
fn reflog_file_read_obeys_byte_budget_and_cancellation() {
    use super::ProbeLimits;
    use super::git_reflog_file::GitReflogFile;
    use super::probe_budget::ProbeBudget;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().canonicalize().unwrap().join("log");
    std::fs::write(&log, b"ten bytes!").unwrap();
    let mut short = ProbeBudget::new(&ProbeLimits {
        max_output_bytes: 9,
        ..ProbeLimits::default()
    })
    .unwrap();
    let mut file = GitReflogFile::open(&log, &mut short).unwrap().unwrap();
    assert!(
        file.read_bounded(&mut short)
            .unwrap_err()
            .contains("output")
    );

    let cancel = Arc::new(AtomicBool::new(false));
    let mut cancelled = ProbeBudget::new(&ProbeLimits {
        cancel: Arc::clone(&cancel),
        ..ProbeLimits::default()
    })
    .unwrap();
    let mut file = GitReflogFile::open(&log, &mut cancelled).unwrap().unwrap();
    cancel.store(true, Ordering::Release);
    assert!(
        file.read_bounded(&mut cancelled)
            .unwrap_err()
            .contains("cancel")
    );
}

#[cfg(unix)]
#[test]
fn same_bytes_with_new_file_version_cannot_pass_second_read() {
    use super::ProbeLimits;
    use super::git_reflog_file::GitReflogFile;
    use super::probe_budget::ProbeBudget;
    use std::time::{Duration, SystemTime};

    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().canonicalize().unwrap().join("log");
    std::fs::write(&log, b"same bytes").unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut held = GitReflogFile::open(&log, &mut budget).unwrap().unwrap();
    assert_eq!(held.read_bounded(&mut budget).unwrap(), b"same bytes");
    // 明确设置可区分的修改时间，避免依赖宿主文件系统时钟粒度或 sleep。
    std::fs::write(&log, b"same bytes").unwrap();
    std::fs::File::open(&log)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_234_567_890))
        .unwrap();
    assert!(!held.matches_path(&log, &mut budget).unwrap());
    assert!(
        held.read_bounded(&mut budget)
            .unwrap_err()
            .contains("changed")
    );
}

#[test]
fn stash_observation_cannot_turn_an_expired_request_into_a_count() {
    let (_temp, project) = two_stashes();
    let error = sample_git_bounded(
        Path::new("git"),
        &project,
        &ProbeLimits {
            timeout: Duration::ZERO,
            ..ProbeLimits::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("deadline"), "{error}");
}
