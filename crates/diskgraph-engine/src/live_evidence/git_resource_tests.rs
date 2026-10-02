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
    for stage in ["head", "upstream"] {
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("git-probe-fixture");
        let marker = temp.path().join("unexpected-command");
        // 在独立 writer 进程中生成并关闭脚本，再回收 writer。
        // 其他并发 fixture 的 fork 不能继承写描述符，避免 Linux ETXTBSY；
        // 不重试生产错误，也不将目标的异常退出断言放宽为任意 I/O 失败。
        let written = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "live_evidence::git_resource_tests::signal_fixture_writer",
                "--quiet",
                "--nocapture",
            ])
            .env_clear()
            .env("DG_GIT_STAGE", stage)
            .env("DG_GIT_PROGRAM", &program)
            .env("DG_GIT_MARKER", &marker)
            .output()
            .unwrap();
        assert!(written.status.success(), "{written:?}");
        let error = sample_git_bounded(&program, temp.path(), &ProbeLimits::default()).unwrap_err();
        assert!(error.contains("normal exit status"), "{stage}: {error}");
        assert!(
            !marker.exists(),
            "{stage}: failed observation started a subsequent command"
        );
    }
}

#[cfg(unix)]
#[test]
fn signal_fixture_writer() {
    use std::os::unix::fs::PermissionsExt;
    let Ok(stage) = std::env::var("DG_GIT_STAGE") else {
        return;
    };
    assert!(matches!(stage.as_str(), "head" | "upstream"));
    let program = std::env::var_os("DG_GIT_PROGRAM").unwrap();
    let marker = std::env::var("DG_GIT_MARKER").unwrap();
    let head = if stage == "head" {
        "kill -TERM $$"
    } else {
        "printf '1111111111111111111111111111111111111111\\n'"
    };
    let upstream = if stage == "upstream" {
        "kill -TERM $$"
    } else {
        "printf '%s\\0%s\\0%s\\0\\n' 'refs/heads/main' '1111111111111111111111111111111111111111' 'refs/remotes/origin/main'"
    };
    let quoted_marker = format!("'{}'", marker.replace('\'', "'\"'\"'"));
    let source = format!(
        "#!/bin/sh\n[ \"$1\" = '--no-pager' ] && [ \"$2\" = '--no-lazy-fetch' ] && [ \"$3\" = '--no-optional-locks' ] || exit 64\nshift 3\ncase \"$*\" in\n 'rev-parse --verify --quiet HEAD^{{commit}}') {head};;\n 'symbolic-ref --quiet --no-recurse HEAD') printf 'refs/heads/main\\n';;\n 'check-ref-format '*) :;;\n 'show-ref --exists refs/stash') exit 2;;\n 'status --porcelain=v1 -z --untracked-files=all'|'stash list --format=%H') :;;\n 'for-each-ref --format=%(refname)%00%(objectname)%00%(upstream)%00 -- refs/heads/main') {upstream};;\n *) : > {quoted_marker}; printf 'true\\n';;\nesac\n"
    );
    std::fs::write(&program, source).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
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
    // HEAD 输出已耗尽额度，后续引用观测必须共用预算并失败，
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
