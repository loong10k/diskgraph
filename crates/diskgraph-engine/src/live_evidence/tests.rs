use super::*;
use std::collections::BTreeMap;
#[cfg(target_os = "macos")]
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "macos")]
#[test]
fn the_usage_sample_sees_the_test_process_own_open_file() {
    let workspace = tempfile::TempDir::with_prefix("dg-live-usage-").unwrap();
    let path = workspace.path().join("held.bin");
    std::fs::write(&path, b"data").unwrap();
    // Keep the handle open for the duration of the sample.
    let _held = File::open(&path).unwrap();
    let sample = sample_process_usage(Path::new("/usr/sbin/lsof"), &[&path]);
    assert_eq!(sample.coverage, UsageCoverage::Full, "{sample:?}");
    assert!(
        !sample.holders.is_empty(),
        "this process holds the file open; the probe must see it"
    );
    assert!(sample.verdict().contains("hold open handles"));
}

#[test]
fn a_missing_probe_is_unobservable_never_nobody() {
    let workspace = tempfile::TempDir::with_prefix("dg-live-unobs-").unwrap();
    let path = workspace.path().join("f.bin");
    std::fs::write(&path, b"data").unwrap();
    let sample = sample_process_usage(Path::new("/nonexistent/lsof"), &[&path]);
    assert!(matches!(
        sample.coverage,
        UsageCoverage::Unobservable { .. }
    ));
    assert!(
        !sample.verdict().contains("no open handles"),
        "an unobservable probe must never read as nobody"
    );
}

#[test]
fn git_samples_dirty_stash_and_upstream_honestly() {
    let git = PathBuf::from("git");
    // Git's installation path differs across macOS, Linux and Windows;
    // use the executable found by the runner's PATH for this host drill.
    assert!(
        Command::new(&git)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success()),
        "git is required for the live repository drill"
    );
    let workspace = tempfile::TempDir::with_prefix("dg-live-git-").unwrap();
    let project = workspace.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    let config = ["-c", "user.email=d@example", "-c", "user.name=d"];
    let run = |args: &[&str]| {
        let mut all: Vec<&str> = config.to_vec();
        all.extend_from_slice(args);
        let output = Command::new(&git)
            .args(&all)
            .current_dir(&project)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["init", "-q"]);
    // Pin the branch name: newer gits default to whatever the host
    // config says, and the upstream wiring below names this branch.
    run(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    // A pristine repository: no commits, so the sample reports exactly
    // that instead of pretending a HEAD exists.
    let pristine = sample_git(&git, &project).unwrap();
    assert!(pristine.head.is_none());

    std::fs::write(project.join("a.txt"), b"one\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "one"]);
    // With no upstream configured, ahead/behind are unknown — and the
    // note says so, never "pushed" (EV-02).
    let sample = sample_git(&git, &project).unwrap();
    assert!(sample.head.is_some());
    assert_eq!(sample.dirty_count, 0);
    assert_eq!(sample.ahead_of_upstream, None);
    assert!(
        sample
            .notes
            .iter()
            .any(|note| note.contains("never reported as pushed"))
    );

    // A dirty working tree counts.
    std::fs::write(project.join("a.txt"), b"two\n").unwrap();
    let sample = sample_git(&git, &project).unwrap();
    assert_eq!(sample.dirty_count, 1);
    run(&["stash", "push", "-q"]);
    let sample = sample_git(&git, &project).unwrap();
    assert_eq!(sample.dirty_count, 0);
    assert_eq!(sample.stash_count, 1);
    run(&["stash", "pop", "-q"]);

    // Point the branch's upstream at a local ref: fully offline.
    run(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    run(&["config", "remote.origin.url", "."]);
    run(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
    ]);
    run(&["config", "branch.main.remote", "origin"]);
    run(&["config", "branch.main.merge", "refs/heads/main"]);
    std::fs::write(project.join("a.txt"), b"three\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "two"]);
    let sample = sample_git(&git, &project).unwrap();
    assert_eq!(sample.ahead_of_upstream, Some(1));
    assert_eq!(sample.behind_upstream, Some(0));
}

#[test]
fn the_polling_watcher_reports_added_modified_removed_and_overflow() {
    let workspace = tempfile::TempDir::with_prefix("dg-live-watch-").unwrap();
    let root = workspace.path().join("tree");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("base.txt"), b"0").unwrap();

    let mut snapshot: WatchSnapshot = BTreeMap::new();
    // The baseline poll establishes the snapshot; its events describe
    // the initial population.
    let baseline = poll_changes(&root, &mut snapshot, 100);
    assert!(
        baseline
            .events
            .iter()
            .any(|event| event.kind == FsEventKind::Appeared)
    );
    assert!(!baseline.rescan_needed);

    // A modification, visible on its own poll.
    std::fs::write(root.join("base.txt"), b"changed").unwrap();
    let report = poll_changes(&root, &mut snapshot, 100);
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == FsEventKind::Modified)
    );
    assert!(!report.rescan_needed);

    // An appearance and a removal share the next poll. A rename arrives
    // as exactly this pair: the sampling can honestly tell no better.
    std::fs::write(root.join("new.txt"), b"n").unwrap();
    std::fs::remove_file(root.join("base.txt")).unwrap();
    let report = poll_changes(&root, &mut snapshot, 100);
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == FsEventKind::Appeared)
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == FsEventKind::Removed)
    );
    assert!(!report.rescan_needed);

    // A burst past the event budget is an overflow that asks for a
    // controlled rescan rather than a trusted partial view.
    for index in 0..5 {
        std::fs::write(root.join(format!("burst-{index}.txt")), b"x").unwrap();
    }
    let report = poll_changes(&root, &mut snapshot, 2);
    assert!(report.overflow);
    assert!(report.rescan_needed);
}
