use super::ProbeLimits;
use super::probe_budget::ProbeBudget;
use super::probe_execution::run_probe;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn limits(bytes: usize, timeout: Duration) -> ProbeLimits {
    ProbeLimits {
        max_output_bytes: bytes,
        timeout,
        ..ProbeLimits::default()
    }
}

#[cfg(unix)]
#[test]
fn head_and_upstream_signal_death_stop_the_entire_sample() {
    use super::{ProbeLimits, sample_git_bounded};
    use std::os::unix::fs::PermissionsExt;
    for stage in ["head", "upstream"] {
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("git-probe-fixture");
        let marker = temp.path().join("unexpected-command");
        let head = if stage == "head" {
            "kill -TERM $$"
        } else {
            "printf 'oid\\n'"
        };
        let upstream = if stage == "upstream" {
            "kill -TERM $$"
        } else {
            "printf 'origin/main\\n'"
        };
        let quoted_marker = format!("'{}'", marker.to_str().unwrap().replace('\'', "'\"'\"'"));
        let source = format!(
            "#!/bin/sh\ncase \"$*\" in\n 'rev-parse HEAD') {head};;\n 'status --porcelain'|'stash list') :;;\n 'rev-parse --abbrev-ref --symbolic-full-name @{{upstream}}') {upstream};;\n *) : > {quoted_marker}; printf 'true\\n';;\nesac\n"
        );
        std::fs::write(&program, source).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = sample_git_bounded(&program, temp.path(), &ProbeLimits::default()).unwrap_err();
        assert!(error.contains("normal exit status"), "{stage}: {error}");
        assert!(
            !marker.exists(),
            "{stage}: failed observation started a subsequent command"
        );
    }
}

#[test]
fn git_head_and_upstream_resource_errors_cannot_be_missing_references() {
    let temp = tempfile::tempdir().unwrap();
    let git = std::path::Path::new("git");
    let run = |args: &[&str]| {
        let output = Command::new(git)
            .args(args)
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    };
    run(&["init", "-q"]);
    let mut silent = Command::new(git);
    silent
        .args(["rev-parse", "--quiet", "--verify", "refs/heads/nonexistent"])
        .current_dir(temp.path());
    super::probe_execution::configure_probe_env(&mut silent);
    let mut zero = ProbeBudget::new(&limits(0, Duration::from_secs(2))).unwrap();
    assert_eq!(
        run_probe(&mut silent, &mut zero).unwrap().exit_code,
        Some(1)
    );
    let one = limits(1, Duration::from_secs(2));
    let error = super::sample_git_bounded(git, temp.path(), &one).unwrap_err();
    assert!(error.contains("output byte limit"), "{error}");
    run(&[
        "-c",
        "user.name=probe",
        "-c",
        "user.email=p@example",
        "commit",
        "--allow-empty",
        "-q",
        "-m",
        "first",
    ]);
    // HEAD 成功输出已耗尽额度，随后无 upstream 的错误 stderr 也必须扣费，
    // 不能将资源失败吞成正常“未配置 upstream”样本。
    let head = Command::new(git)
        .args(["rev-parse", "HEAD"])
        .current_dir(temp.path())
        .output()
        .unwrap()
        .stdout
        .len();
    let exact_head = limits(head, Duration::from_secs(2));
    let error = super::sample_git_bounded(git, temp.path(), &exact_head).unwrap_err();
    assert!(error.contains("output byte limit"), "{error}");
    let cancelled = ProbeLimits::default();
    cancelled.cancel.store(true, Ordering::Release);
    assert!(
        super::sample_git_bounded(git, temp.path(), &cancelled)
            .unwrap_err()
            .contains("cancelled")
    );
}
