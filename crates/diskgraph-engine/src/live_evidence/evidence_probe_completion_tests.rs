//! 显式清理之后仍检查原期限/取消，不交付可复用的 Git 输入额度。

use super::ProbeLimits;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_view::GitView;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn check_terminal(cancellation: bool) {
    let fixture = GitIsolationFixture::new("sha1");
    let limits = ProbeLimits {
        timeout: Duration::from_secs(60),
        ..ProbeLimits::default()
    };
    let mut probe = ProbeBudget::new(&limits).unwrap();
    let view = GitView::prepare(
        Path::new("git"),
        fixture.path(),
        &mut probe,
        GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
    .unwrap();
    let output = view
        .run(&["rev-parse", "--absolute-git-dir"], &mut probe)
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    let private_repo = PathBuf::from(std::str::from_utf8(&output.stdout).unwrap().trim_end());
    let private_root = private_repo.parent().unwrap();
    assert!(private_root.exists());
    if cancellation {
        limits.cancel.store(true, Ordering::Release);
    } else {
        probe.expire_for_test();
    }
    let error = view
        .complete_with_metadata(Ok(7), &mut probe)
        .err()
        .unwrap();
    assert!(
        error.contains(if cancellation {
            "cancelled"
        } else {
            "deadline"
        }),
        "{error}"
    );
    assert!(
        !private_root.exists(),
        "terminal failure prevented explicit cleanup"
    );
}

#[test]
fn cleaned_view_does_not_return_metadata_after_deadline() {
    check_terminal(false);
}

#[test]
fn cleaned_view_does_not_return_metadata_after_cancellation() {
    check_terminal(true);
}
