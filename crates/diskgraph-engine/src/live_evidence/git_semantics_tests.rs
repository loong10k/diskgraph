use super::sample_git;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run_git(project: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=fixture", "-c", "user.email=f@example"])
        .args(args)
        .current_dir(project)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

fn repository(committed: bool) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    std::fs::create_dir(&project).unwrap();
    run_git(&project, &["init", "-q"]);
    run_git(&project, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    if committed {
        run_git(&project, &["commit", "--allow-empty", "-q", "-m", "base"]);
    }
    (temp, project)
}

fn tracking(project: &Path) {
    run_git(project, &["config", "remote.origin.url", "."]);
    run_git(
        project,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    run_git(project, &["config", "branch.main.remote", "origin"]);
    run_git(project, &["config", "branch.main.merge", "refs/heads/main"]);
}

#[test]
fn unborn_repository_counts_each_untracked_and_staged_file() {
    let (_temp, project) = repository(false);
    std::fs::create_dir(project.join("nested")).unwrap();
    std::fs::write(project.join("nested/one"), b"one").unwrap();
    std::fs::write(project.join("nested/two"), b"two").unwrap();
    let sample = sample_git(Path::new("git"), &project).unwrap();
    assert_eq!(sample.head, None);
    assert_eq!(sample.dirty_count, 2, "{sample:?}");
    run_git(&project, &["add", "."]);
    let sample = sample_git(Path::new("git"), &project).unwrap();
    assert_eq!(sample.head, None);
    assert_eq!(sample.dirty_count, 2, "{sample:?}");
    assert_eq!(sample.stash_count, 0);
}

#[test]
fn committed_repository_counts_untracked_files_not_directory_display_rows() {
    let (_temp, project) = repository(true);
    std::fs::create_dir(project.join("nested")).unwrap();
    for name in ["one", "two", "three"] {
        std::fs::write(project.join("nested").join(name), b"x").unwrap();
    }
    assert_eq!(
        sample_git(Path::new("git"), &project).unwrap().dirty_count,
        3
    );
}

#[test]
fn corrupt_current_branch_is_an_error_not_an_unborn_repository() {
    let (_temp, project) = repository(true);
    std::fs::write(project.join(".git/refs/heads/main"), b"invalid-oid\n").unwrap();
    assert!(sample_git(Path::new("git"), &project).is_err());
}

#[test]
fn corrupt_upstream_is_an_error_not_missing_configuration() {
    let (_temp, project) = repository(true);
    tracking(&project);
    let refs = project.join(".git/refs/remotes/origin");
    std::fs::create_dir_all(&refs).unwrap();
    std::fs::write(refs.join("main"), b"invalid-oid\n").unwrap();
    assert!(sample_git(Path::new("git"), &project).is_err());
}

#[test]
fn existing_references_with_missing_commit_objects_are_errors() {
    for name in ["refs/heads/main", "refs/remotes/origin/main", "refs/stash"] {
        let (_temp, project) = repository(true);
        tracking(&project);
        let path = project.join(".git").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"1111111111111111111111111111111111111111\n").unwrap();
        assert!(sample_git(Path::new("git"), &project).is_err(), "{name}");
    }
}

#[test]
fn dangling_symbolic_references_are_errors_not_absence() {
    for name in ["refs/heads/main", "refs/remotes/origin/main", "refs/stash"] {
        let (_temp, project) = repository(true);
        tracking(&project);
        run_git(&project, &["symbolic-ref", name, "refs/heads/absent"]);
        assert!(sample_git(Path::new("git"), &project).is_err(), "{name}");
    }
}

#[test]
fn corrupt_stash_is_an_error_not_zero_stashes() {
    let (_temp, project) = repository(true);
    std::fs::write(project.join(".git/refs/stash"), b"invalid-oid\n").unwrap();
    assert!(sample_git(Path::new("git"), &project).is_err());
}

#[test]
fn detached_head_keeps_dirty_observations_and_unknown_upstream() {
    let (_temp, project) = repository(true);
    run_git(&project, &["checkout", "--detach", "-q"]);
    std::fs::write(project.join("dirty"), b"x").unwrap();
    let sample = sample_git(Path::new("git"), &project).unwrap();
    assert!(sample.head.is_some());
    assert_eq!(sample.dirty_count, 1);
    assert_eq!(sample.ahead_of_upstream, None);
    assert_eq!(sample.behind_upstream, None);
    assert!(
        sample
            .notes
            .iter()
            .any(|note| note.contains("detached HEAD"))
    );
}

#[test]
fn sha256_repository_keeps_full_oids_and_correct_commit_counts() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("sha256");
    std::fs::create_dir(&project).unwrap();
    run_git(&project, &["init", "-q", "--object-format=sha256"]);
    run_git(&project, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    run_git(&project, &["commit", "--allow-empty", "-q", "-m", "base"]);
    tracking(&project);
    run_git(
        &project,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    run_git(&project, &["commit", "--allow-empty", "-q", "-m", "ahead"]);
    let sample = sample_git(Path::new("git"), &project).unwrap();
    assert_eq!(sample.head.as_ref().unwrap().len(), 64);
    assert_eq!(sample.dirty_count, 0);
    assert_eq!(sample.ahead_of_upstream, Some(1));
    assert_eq!(sample.behind_upstream, Some(0));
}

#[test]
fn missing_tracking_ref_is_reported_as_unavailable_not_unconfigured() {
    let (_temp, project) = repository(true);
    tracking(&project);
    let sample = sample_git(Path::new("git"), &project).unwrap();
    assert_eq!(sample.ahead_of_upstream, None);
    assert_eq!(sample.behind_upstream, None);
    assert!(
        sample
            .notes
            .iter()
            .any(|note| { note.contains("upstream reference") && note.contains("unavailable") }),
        "{sample:?}"
    );
}

#[cfg(unix)]
#[test]
fn newline_and_rename_paths_have_one_status_per_file() {
    let (_temp, project) = repository(true);
    let newline = project.join("line\nbreak");
    let native = project.join("原生路径");
    std::fs::write(&newline, b"line").unwrap();
    std::fs::write(&native, b"native").unwrap();
    std::fs::write(project.join("-option"), b"option").unwrap();
    run_git(&project, &["add", "."]);
    run_git(&project, &["commit", "-q", "-m", "names"]);
    std::fs::rename(newline, project.join("new\nname")).unwrap();
    std::fs::write(native, b"changed").unwrap();
    run_git(&project, &["add", "."]);
    assert_eq!(
        sample_git(Path::new("git"), &project).unwrap().dirty_count,
        2
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_non_utf8_status_paths_are_counted_without_text_conversion() {
    use std::os::unix::ffi::OsStringExt;
    let (_temp, project) = repository(true);
    let path = project.join(std::ffi::OsString::from_vec(b"native-\xff".to_vec()));
    std::fs::write(path, b"native").unwrap();
    assert_eq!(
        sample_git(Path::new("git"), &project).unwrap().dirty_count,
        1
    );
}

#[cfg(unix)]
fn assert_bad_fixture(stage: &str, diagnostic: &str) {
    let (temp, project) = repository(true);
    let program = temp.path().join("git-fixture");
    let oid = "1111111111111111111111111111111111111111";
    let upstream = "2222222222222222222222222222222222222222";
    let head = if stage == "oid" { "invalid" } else { oid };
    let head_command = if stage == "head_changed" {
        let state = temp.path().join("head-observed");
        let quoted = format!("'{}'", state.to_str().unwrap().replace('\'', "'\"'\"'"));
        format!(
            "if test -e {quoted}; then printf '{upstream}\\n'; else : > {quoted}; printf '{oid}\\n'; fi"
        )
    } else {
        format!("printf '{head}\\n'")
    };
    let counter = match stage {
        "count" => "not-a-count",
        "overflow" => "18446744073709551616",
        _ => "0",
    };
    let status = if stage == "status" {
        "printf '?? unterminated'"
    } else {
        ":"
    };
    let metadata = if stage == "upstream" {
        "printf 'fixture failure\\n' >&2; exit 17".to_string()
    } else {
        format!("printf '%s\\0%s\\0%s\\0\\n' 'refs/heads/main' '{oid}' 'refs/remotes/origin/main'")
    };
    let old_upstream = if stage == "upstream" {
        "printf 'fixture failure\\n' >&2; exit 17"
    } else {
        "printf 'refs/remotes/origin/main\\n'"
    };
    let stash_exists = if stage == "unsupported" {
        "printf 'unknown option: exists\\n' >&2; exit 129"
    } else {
        "exit 2"
    };
    let source = format!(
        "#!/bin/sh\n[ \"$1\" = '--no-pager' ] && [ \"$2\" = '--no-lazy-fetch' ] && [ \"$3\" = '--no-optional-locks' ] || exit 64\nshift 3\ncase \"$*\" in\n\
        config*|var*) exec git --no-pager --no-lazy-fetch --no-optional-locks \"$@\";;\n\
        'rev-parse HEAD'|'rev-parse --verify --quiet HEAD^{{commit}}') {head_command};;\n\
        'symbolic-ref --quiet HEAD'|'symbolic-ref --quiet --no-recurse HEAD') printf 'refs/heads/main\\n';;\n\
        'check-ref-format '*) :;;\n\
        'show-ref --exists refs/stash') {stash_exists};;\n\
        'show-ref --exists refs/remotes/origin/main') :;;\n\
        'status --porcelain'|'status --porcelain=v1 -z --untracked-files=all') {status};;\n\
        'stash list'|'stash list --format=%H') :;;\n\
        'rev-parse --abbrev-ref --symbolic-full-name @{{upstream}}') {old_upstream};;\n\
        'for-each-ref --format=%(refname)%00%(objectname)%00%(upstream)%00 -- refs/heads/main') {metadata};;\n\
        'rev-parse --verify --quiet refs/remotes/origin/main^{{commit}}') printf '{upstream}\\n';;\n\
        'rev-list --count '*) printf '{counter}\\n';;\n\
        *) printf 'unexpected fixture command\\n' >&2; exit 23;;\nesac\n"
    );
    // 单独 writer 退出后才执行目标，避免并发 fork 继承脚本写 FD 的 ETXTBSY。
    let written = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "live_evidence::git_semantics_tests::fixture_writer",
            "--quiet",
            "--nocapture",
        ])
        .env_clear()
        .env("DG_GIT_FIXTURE_PATH", &program)
        .env("DG_GIT_FIXTURE_SOURCE", source)
        .output()
        .unwrap();
    assert!(written.status.success(), "{written:?}");
    let error = sample_git(&program, &project).unwrap_err();
    assert!(error.contains(diagnostic), "{stage}: {error}");
}

#[cfg(unix)]
#[test]
fn fixture_writer() {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("DG_GIT_FIXTURE_PATH") else {
        return;
    };
    let source = std::env::var("DG_GIT_FIXTURE_SOURCE").unwrap();
    std::fs::write(&path, source).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
fn invalid_head_oid_is_a_format_error() {
    assert_bad_fixture("oid", "invalid object id");
}

#[cfg(unix)]
#[test]
fn failed_upstream_lookup_is_not_missing_configuration() {
    assert_bad_fixture("upstream", "fixture failure");
}

#[cfg(unix)]
#[test]
fn malformed_commit_count_is_an_error() {
    assert_bad_fixture("count", "invalid commit count");
}

#[cfg(unix)]
#[test]
fn overflowing_commit_count_is_an_error() {
    assert_bad_fixture("overflow", "invalid commit count");
}

#[cfg(unix)]
#[test]
fn unterminated_status_is_an_error() {
    assert_bad_fixture("status", "invalid status record");
}

#[cfg(unix)]
#[test]
fn unsupported_reference_interface_is_an_error_not_a_fallback() {
    assert_bad_fixture("unsupported", "Git 2.46+ required");
}

#[cfg(unix)]
#[test]
fn changed_head_cannot_publish_a_mixed_sample() {
    assert_bad_fixture("head_changed", "HEAD changed during Git sampling");
}
