//! Unix 受信 Git 故障程序：真实语义/复核/清理错误后不得启动下一目标。

use super::git_executable::GitExecutable;
use super::git_isolation_fixture::GitIsolationFixture;
use super::probe_budget::ProbeBudget;
use super::{EvidenceProbeSession, ProbeLimits, UsageCoverage};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\"'\"'"))
}

fn failed_git_observation(stage: &str) {
    let fixture = GitIsolationFixture::new("sha1");
    let marker = fixture.path().parent().unwrap().join("private-path");
    let forbidden_marker = fixture
        .path()
        .parent()
        .unwrap()
        .join("unexpected-next-probe");
    let limits = ProbeLimits {
        timeout: Duration::from_secs(60),
        ..ProbeLimits::default()
    };
    let mut probe = ProbeBudget::new(&limits).unwrap();
    let actual = GitExecutable::resolve(Path::new("git"), &mut probe)
        .unwrap()
        .path()
        .to_path_buf();
    let fault = match stage {
        "observe" => "printf 'broken\\0'; exit 0".to_owned(),
        "exit" => "printf 'fixture command failure\\n' >&2; exit 47".to_owned(),
        "verify" => format!(
            "printf '\\n# fixture mutation\\n' >> {}",
            quote(&fixture.path().join(".git/config"))
        ),
        "cleanup" => {
            "mkdir \"$GIT_DIR/blocked\" || exit 48\nchmod 000 \"$GIT_DIR/blocked\" || exit 49"
                .to_owned()
        }
        _ => panic!("unknown fixture stage"),
    };
    let program = fixture.script("faulting-git", &format!(
        "#!/bin/sh\nif [ \"$4\" = status ]; then\n printf '%s\\n' \"$GIT_DIR\" > {}\n {}\nfi\nexec {} \"$@\"\n", quote(&marker), fault, quote(&actual)));
    let mut session = EvidenceProbeSession::new(&limits).unwrap();
    let result = session.sample_git(&program, fixture.path());
    // 恢复权限并安全删除隔离私有数据必须发生在任何结果断言前。
    let private_repo = PathBuf::from(
        std::fs::read_to_string(&marker)
            .expect("fault program ran")
            .trim_end(),
    );
    let root = private_repo.parent().unwrap();
    let cleanup_failed = if root.exists() {
        std::fs::remove_dir_all(root).is_err()
    } else {
        false
    };
    if root.exists() {
        std::fs::set_permissions(
            private_repo.join("blocked"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    let error = result.unwrap_err();
    match stage {
        "observe" => assert!(error.contains("invalid status record"), "{error}"),
        "exit" => assert!(error.contains("fixture command failure"), "{error}"),
        "verify" => assert!(error.contains("changed"), "{error}"),
        "cleanup" => {
            assert!(
                cleanup_failed,
                "fixture did not cause actual cleanup failure"
            );
            assert!(error.contains("cleanup"), "{error}");
        }
        _ => unreachable!("validated above"),
    }
    assert!(
        session.resources_for_test().1.is_none(),
        "failed view replenished metadata"
    );
    let forbidden = fixture.script(
        "must-not-start",
        &format!(
            "#!/bin/sh\nprintf x > {}\nexit 47\n",
            quote(&forbidden_marker)
        ),
    );
    let later = session.sample_process_usage(&forbidden, &[fixture.path()]);
    assert!(matches!(later.coverage, UsageCoverage::Unobservable { reason } if reason == error));
    assert_eq!(
        session.sample_git(&forbidden, fixture.path()).unwrap_err(),
        error
    );
    assert!(
        !forbidden_marker.exists(),
        "closed session launched another probe"
    );
}

#[test]
fn git_output_semantic_failure_prevents_future_program_start() {
    failed_git_observation("observe");
}

#[test]
fn git_nonzero_exit_failure_prevents_future_program_start() {
    failed_git_observation("exit");
}

#[test]
fn git_source_verification_failure_prevents_future_program_start() {
    failed_git_observation("verify");
}

#[test]
fn git_cleanup_failure_prevents_future_program_start() {
    failed_git_observation("cleanup");
}
