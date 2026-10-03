//! D29 真实进程共享预算回归；仅使用独占临时目录与受信测试程序。

use super::git_isolation_fixture::GitIsolationFixture;
use super::{EvidenceProbeSession, ProbeLimits, UsageCoverage};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[cfg(unix)]
fn process_fixture(body: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let target = root.join("observed");
    std::fs::write(&target, b"fixture").unwrap();
    let program = root.join("trusted-probe");
    // 独立 writer 退出后启动脚本，避免 Linux 并发 fork 继承未关闭 writer 而 ETXTBSY。
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "live_evidence::git_isolation_tests::script_writer_fixture",
            "--quiet",
        ])
        .env_clear()
        .env("DG_ISOLATION_SCRIPT_PATH", &program)
        .env("DG_ISOLATION_SCRIPT_BYTES", format!("#!/bin/sh\n{body}\n"))
        .output()
        .unwrap();
    assert!(output.status.success(), "script writer: {output:?}");
    (temp, program, target)
}

#[cfg(unix)]
fn output_bytes(target: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt;
    b"p123\0cfixture\0n".len() + target.as_os_str().as_bytes().len() + b"\0\n".len()
}

#[cfg(unix)]
#[test]
fn task_output_budget_is_shared_across_process_samples() {
    let (_temp, program, target) = process_fixture("printf 'p123\\0cfixture\\0n%s\\0\\n' \"$5\"");
    let limits = ProbeLimits {
        max_output_bytes: output_bytes(&target) * 2 - 1,
        ..ProbeLimits::default()
    };
    let mut session = EvidenceProbeSession::new(&limits).unwrap();
    let first = session.sample_process_usage(&program, &[&target]);
    assert!(matches!(first.coverage, UsageCoverage::Partial { .. }));
    assert_eq!(first.holders.len(), 1);
    let second = session.sample_process_usage(&program, &[&target]);
    assert!(
        matches!(second.coverage, UsageCoverage::Unobservable { ref reason } if reason.contains("output byte limit")),
        "second observation obtained a fresh task budget: {second:?}"
    );
}

#[cfg(unix)]
#[test]
fn failed_task_cannot_become_full_via_empty_process_request() {
    let (_temp, program, target) = process_fixture("printf broken");
    let limits = ProbeLimits::default();
    let mut session = EvidenceProbeSession::new(&limits).unwrap();
    let failed = session.sample_process_usage(&program, &[&target]);
    assert!(matches!(
        failed.coverage,
        UsageCoverage::Unobservable { .. }
    ));
    let later = session.sample_process_usage(&program, &[]);
    assert!(
        matches!(later.coverage, UsageCoverage::Unobservable { .. }),
        "failed task became complete: {later:?}"
    );
}

fn ample_limits() -> ProbeLimits {
    ProbeLimits {
        timeout: Duration::from_secs(60),
        ..ProbeLimits::default()
    }
}

#[test]
fn successive_git_observations_consume_original_balances() {
    let fixture = GitIsolationFixture::new("sha1");
    let mut session = EvidenceProbeSession::new(&ample_limits()).unwrap();
    let before = session.resources_for_test();
    let first = session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap();
    let middle = session.resources_for_test();
    let second = session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap();
    let after = session.resources_for_test();
    assert_eq!(first.head, second.head);
    assert!(before.0 > middle.0 && middle.0 > after.0);
    for index in [0, 1] {
        let values = [before.1.unwrap(), middle.1.unwrap(), after.1.unwrap()];
        let get = |pair: (usize, usize)| if index == 0 { pair.0 } else { pair.1 };
        assert!(get(values[0]) > get(values[1]) && get(values[1]) > get(values[2]));
    }
}

fn git_cost(fixture: &GitIsolationFixture) -> (usize, usize, usize) {
    let mut session = EvidenceProbeSession::new(&ample_limits()).unwrap();
    let before = session.resources_for_test();
    session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap();
    let after = session.resources_for_test();
    (
        before.0 - after.0,
        before.1.unwrap().0 - after.1.unwrap().0,
        before.1.unwrap().1 - after.1.unwrap().1,
    )
}

#[test]
fn second_git_cannot_reset_output_quota() {
    let fixture = GitIsolationFixture::new("sha1");
    let used = git_cost(&fixture).0;
    assert!(used > 0);
    let limits = ProbeLimits {
        max_output_bytes: 2 * used - 1,
        ..ample_limits()
    };
    let mut session = EvidenceProbeSession::new(&limits).unwrap();
    session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap();
    let error = session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap_err();
    assert!(error.contains("output byte limit"), "{error}");
    assert!(session.resources_for_test().1.is_none());
    assert_eq!(
        session
            .sample_git(Path::new("not-a-program"), fixture.path())
            .unwrap_err(),
        error
    );
}

fn assert_metadata_cumulative(bytes: bool) {
    let fixture = GitIsolationFixture::new("sha1");
    let (_, used_bytes, used_entries) = git_cost(&fixture);
    assert!(used_bytes > 0 && used_entries > 0);
    let mut session = EvidenceProbeSession::new(&ample_limits()).unwrap();
    session.metadata_limits_for_test(
        if bytes { 2 * used_bytes - 1 } else { 64 << 20 },
        if bytes { 32_768 } else { 2 * used_entries - 1 },
    );
    session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap();
    let error = session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap_err();
    let kind = if bytes { "byte" } else { "entry" };
    assert!(error.contains(&format!("metadata {kind} limit")), "{error}");
    assert!(
        session.resources_for_test().1.is_none(),
        "failed metadata returned to session"
    );
    let later = session.sample_process_usage(Path::new("not-a-program"), &[]);
    assert!(matches!(later.coverage, UsageCoverage::Unobservable { reason } if reason == error));
}

#[test]
fn git_metadata_bytes_accumulate_across_observations() {
    assert_metadata_cumulative(true);
}

#[test]
fn git_metadata_entries_accumulate_across_observations() {
    assert_metadata_cumulative(false);
}

#[test]
fn deadline_is_creation_bound_and_failure_stays_latched() {
    let mut session = EvidenceProbeSession::new(&ample_limits()).unwrap();
    session.expire_for_test();
    let error = session
        .sample_git(Path::new("not-a-program"), Path::new("not-a-project"))
        .unwrap_err();
    assert!(error.contains("deadline"), "{error}");
    let later = session.sample_process_usage(Path::new("not-a-program"), &[]);
    assert!(matches!(later.coverage, UsageCoverage::Unobservable { reason } if reason == error));
}

#[test]
fn cancellation_cannot_be_reset_to_reopen_session() {
    let limits = ample_limits();
    let mut session = EvidenceProbeSession::new(&limits).unwrap();
    limits.cancel.store(true, Ordering::Release);
    let error = session
        .sample_git(Path::new("not-a-program"), Path::new("not-a-project"))
        .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    limits.cancel.store(false, Ordering::Release);
    let later = session.sample_process_usage(Path::new("not-a-program"), &[]);
    assert!(matches!(later.coverage, UsageCoverage::Unobservable { reason } if reason == error));
}

#[test]
fn invalid_cancelled_or_expired_configuration_is_rejected_at_creation() {
    let limits = ProbeLimits {
        max_output_bytes: (64 << 20) + 1,
        ..ample_limits()
    };
    assert!(
        EvidenceProbeSession::new(&limits)
            .err()
            .unwrap()
            .contains("invalid")
    );
    let limits = ProbeLimits {
        timeout: Duration::ZERO,
        ..ample_limits()
    };
    assert!(
        EvidenceProbeSession::new(&limits)
            .err()
            .unwrap()
            .contains("deadline")
    );
    let limits = ample_limits();
    limits.cancel.store(true, Ordering::Release);
    assert!(
        EvidenceProbeSession::new(&limits)
            .err()
            .unwrap()
            .contains("cancelled")
    );
}

#[test]
fn semantic_git_prepare_failure_closes_both_observation_kinds() {
    let fixture = GitIsolationFixture::new("sha1");
    let other = String::from_utf8(
        super::git_native_path::tool_bytes(&fixture.path().join("other-worktree")).unwrap(),
    )
    .unwrap();
    fixture.git(&["config", "core.worktree", &other]);
    let mut session = EvidenceProbeSession::new(&ample_limits()).unwrap();
    let error = session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap_err();
    assert!(error.contains("unsupported Git core.worktree"), "{error}");
    assert!(session.resources_for_test().1.is_none());
    assert_eq!(
        session
            .sample_git(Path::new("not-a-program"), fixture.path())
            .unwrap_err(),
        error
    );
    let later = session.sample_process_usage(Path::new("not-a-program"), &[]);
    assert!(matches!(later.coverage, UsageCoverage::Unobservable { reason } if reason == error));
}

#[cfg(unix)]
#[test]
fn shared_output_crosses_git_and_process_in_both_orders() {
    let fixture = GitIsolationFixture::new("sha1");
    let git_bytes = git_cost(&fixture).0;
    let (_temp, program, target) = process_fixture("printf 'p123\\0cfixture\\0n%s\\0\\n' \"$5\"");
    let process_bytes = output_bytes(&target);
    let limits = ProbeLimits {
        max_output_bytes: git_bytes + process_bytes - 1,
        ..ample_limits()
    };
    let mut session = EvidenceProbeSession::new(&limits).unwrap();
    session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap();
    let failed = session.sample_process_usage(&program, &[&target]);
    assert!(
        matches!(failed.coverage, UsageCoverage::Unobservable { reason } if reason.contains("output byte limit"))
    );
    let mut session = EvidenceProbeSession::new(&limits).unwrap();
    let first = session.sample_process_usage(&program, &[&target]);
    assert!(matches!(first.coverage, UsageCoverage::Partial { .. }));
    let error = session
        .sample_git(Path::new("git"), fixture.path())
        .unwrap_err();
    assert!(error.contains("output byte limit"), "{error}");
}

#[cfg(unix)]
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
#[test]
fn malformed_process_sample_prevents_git_program_start() {
    let (temp, program, target) = process_fixture("printf broken");
    let marker = temp.path().join("unexpected-start");
    let fixture = GitIsolationFixture::new("sha1");
    let git = fixture.script(
        "must-not-start",
        &format!("#!/bin/sh\nprintf x > {}\nexit 47\n", quote(&marker)),
    );
    let mut session = EvidenceProbeSession::new(&ample_limits()).unwrap();
    let failed = session.sample_process_usage(&program, &[&target]);
    let UsageCoverage::Unobservable { reason } = failed.coverage else {
        panic!("not unobservable")
    };
    assert_eq!(
        session.sample_git(&git, fixture.path()).unwrap_err(),
        reason
    );
    assert!(!marker.exists(), "closed session started Git");
}

#[cfg(unix)]
#[test]
fn old_independent_wrappers_and_distinct_sessions_remain_independent() {
    let (_temp, program, target) = process_fixture("printf 'p123\\0cfixture\\0n%s\\0\\n' \"$5\"");
    let limits = ProbeLimits {
        max_output_bytes: output_bytes(&target),
        ..ample_limits()
    };
    for _ in 0..2 {
        let sample = super::sample_process_usage_bounded(&program, &[&target], &limits);
        assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
        assert_eq!(sample.holders.len(), 1);
    }
    let cancelled_limits = ample_limits();
    let mut cancelled = EvidenceProbeSession::new(&cancelled_limits).unwrap();
    let mut other = EvidenceProbeSession::new(&ample_limits()).unwrap();
    cancelled_limits.cancel.store(true, Ordering::Release);
    assert!(matches!(
        cancelled
            .sample_process_usage(&program, &[&target])
            .coverage,
        UsageCoverage::Unobservable { .. }
    ));
    assert!(matches!(
        other.sample_process_usage(&program, &[&target]).coverage,
        UsageCoverage::Partial { .. }
    ));
}

#[cfg(unix)]
#[test]
fn stderr_is_charged_even_when_only_stdout_is_interpreted() {
    let (_temp, program, target) =
        process_fixture("printf warning >&2\nprintf 'p123\\0cfixture\\0n%s\\0\\n' \"$5\"");
    let limits = ProbeLimits {
        max_output_bytes: 2 * (output_bytes(&target) + b"warning".len()) - 1,
        ..ample_limits()
    };
    let mut session = EvidenceProbeSession::new(&limits).unwrap();
    assert!(matches!(
        session.sample_process_usage(&program, &[&target]).coverage,
        UsageCoverage::Partial { .. }
    ));
    assert!(
        matches!(session.sample_process_usage(&program, &[&target]).coverage, UsageCoverage::Unobservable { reason } if reason.contains("output byte limit"))
    );
}
