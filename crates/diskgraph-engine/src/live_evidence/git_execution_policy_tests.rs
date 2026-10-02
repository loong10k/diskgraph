//! 真实本地 Git 夹具，检验采样命令的可选索引写入与 promisor 懒获取边界。

use super::sample_git;
use std::fs::{FileTimes, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

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

fn repository() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    std::fs::create_dir(&project).unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(project.join("tracked"), b"unchanged content\n").unwrap();
    git(&project, &["add", "tracked"]);
    git(&project, &["commit", "-q", "-m", "base"]);
    (temp, project)
}

#[test]
fn clean_stat_refresh_does_not_rewrite_the_git_index() {
    let (_temp, project) = repository();
    let index = project.join(".git/index");
    let lock = project.join(".git/index.lock");
    let tracked = project.join("tracked");
    assert!(!lock.exists());

    // 内容保持一致，只将工作树 stat 与 index 缓存分离；两个时间均与当前年相距很远。
    OpenOptions::new()
        .write(true)
        .open(&tracked)
        .unwrap()
        .set_times(
            FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200)),
        )
        .unwrap();
    OpenOptions::new()
        .write(true)
        .open(&index)
        .unwrap()
        .set_times(
            FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_009_843_200)),
        )
        .unwrap();
    let before = std::fs::read(&index).unwrap();
    let modified = std::fs::metadata(&index).unwrap().modified().unwrap();
    let sample = sample_git(Path::new("git"), &project).unwrap();
    assert_eq!(sample.dirty_count, 0);
    assert!(
        std::fs::read(&index).unwrap() == before,
        "Git rewrote index bytes"
    );
    assert_eq!(
        std::fs::metadata(&index).unwrap().modified().unwrap(),
        modified,
        "Git rewrote the index even though contents were clean"
    );
    assert!(!lock.exists(), "Git left index.lock behind");
}

fn remote_ext_escape(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '%' => encoded.push_str("%%"),
            ' ' => encoded.push_str("% "),
            _ => encoded.push(character),
        }
    }
    encoded
}

#[test]
fn promisor_missing_head_does_not_invoke_a_remote_helper() {
    let (_temp, project) = repository();
    let marker = project.parent().unwrap().join("lazy fetch marker");
    let executable = std::env::current_exe().unwrap();
    let program = remote_ext_escape(executable.to_str().unwrap());
    let marker_arg = remote_ext_escape(marker.to_str().unwrap());
    let url = format!(
        "ext::{program} --exact live_evidence::git_execution_policy_tests::promisor_helper_fixture --nocapture --quiet -- {marker_arg}"
    );
    git(&project, &["config", "remote.origin.url", &url]);
    git(&project, &["config", "remote.origin.promisor", "true"]);
    git(
        &project,
        &["config", "remote.origin.partialclonefilter", "blob:none"],
    );
    git(&project, &["config", "protocol.ext.allow", "always"]);
    let oid = String::from_utf8(git(&project, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim_end_matches('\n')
        .to_owned();
    let object = project.join(".git/objects").join(&oid[..2]).join(&oid[2..]);
    assert!(
        object.is_file(),
        "fixture expects an isolated loose HEAD commit"
    );
    std::fs::remove_file(object).unwrap();

    // 先直接证明 remote-ext/argv marker 夹具真实可被 Git promisor 路径启动。
    let direct = Command::new("git")
        .args(["cat-file", "-t", &oid])
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        !direct.status.success(),
        "helper must not provide an object"
    );
    assert!(
        marker.exists(),
        "fixture did not exercise the local remote helper: {direct:?}"
    );
    std::fs::remove_file(&marker).unwrap();

    assert!(sample_git(Path::new("git"), &project).is_err());
    assert!(
        !marker.exists(),
        "Git sample invoked the promisor remote helper"
    );
}

#[test]
fn promisor_helper_fixture() {
    let args: Vec<_> = std::env::args_os().collect();
    let Some(separator) = args.iter().position(|argument| argument == "--") else {
        return;
    };
    let marker = args.get(separator + 1).expect("remote-ext marker path");
    std::fs::write(marker, b"helper invoked").unwrap();
    std::process::exit(47);
}
