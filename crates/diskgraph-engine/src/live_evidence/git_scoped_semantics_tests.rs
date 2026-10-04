//! D35 / EC-04 私有完整捕获必须保持的真实 Git 差分；来源：原生 Rust 采样契约。
//! 使用独立命令及手工期望作正控制，未实现的 scoped 能力不能靠修改信任或忽略策略凑结果。

use super::ProbeLimits;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_scoped_fixture::{STATUS, prepare};
use super::probe_budget::ProbeBudget;
use std::time::{Duration, SystemTime};

fn assert_private_output(fixture: &GitIsolationFixture, args: &[&str], expected: &[u8]) {
    let native = fixture.git(args);
    assert_eq!(native, expected, "native Git positive control");
    let before = fixture.metadata();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = prepare(fixture.path(), &mut probe).unwrap();
    let result = view.run(args, &mut probe);
    let verified = view.verify(&mut probe);
    let output = view.complete(result).unwrap();
    verified.unwrap();
    probe.check().unwrap();
    assert_eq!(output.exit_code, Some(0), "private Git: {output:?}");
    assert_eq!(
        output.stdout, native,
        "capture changed ordinary Git semantics"
    );
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn scoped_sha1_and_sha256_dirty_untracked_and_ignore_match_real_git() {
    for format in ["sha1", "sha256"] {
        let fixture = GitIsolationFixture::new(format);
        std::fs::write(fixture.path().join(".gitignore"), b"*.ignored\n").unwrap();
        fixture.git(&["add", ".gitignore"]);
        fixture.git(&["commit", "-q", "-m", "ignore"]);
        std::fs::write(fixture.path().join("tracked"), b"changed dirty content\n").unwrap();
        std::fs::write(fixture.path().join("visible"), b"untracked\n").unwrap();
        std::fs::write(fixture.path().join("hidden.ignored"), b"ignored\n").unwrap();
        assert_private_output(&fixture, STATUS, b" M tracked\0?? visible\0");
    }
}

#[test]
fn scoped_unborn_staged_and_untracked_files_keep_status_records() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["checkout", "--orphan", "unborn"]);
    fixture.git(&["rm", "-q", "--cached", "tracked"]);
    std::fs::write(fixture.path().join("staged"), b"staged\n").unwrap();
    fixture.git(&["add", "staged"]);
    assert_private_output(&fixture, STATUS, b"A  staged\0?? tracked\0");
}

#[test]
fn scoped_stash_reference_and_log_preserve_real_local_history() {
    let fixture = GitIsolationFixture::new("sha1");
    for bytes in [b"first\n".as_slice(), b"second\n"] {
        std::fs::write(fixture.path().join("tracked"), bytes).unwrap();
        fixture.git(&["stash", "push", "-q"]);
    }
    let oids = fixture.git(&["stash", "list", "--format=%H"]);
    assert_eq!(
        oids.split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count(),
        2
    );
    assert_private_output(&fixture, &["stash", "list", "--format=%H"], &oids);
}

#[test]
fn scoped_builtin_crlf_and_attributes_keep_clean_status() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "core.autocrlf", "true"]);
    std::fs::write(fixture.path().join(".gitattributes"), b"tracked text\n").unwrap();
    std::fs::write(fixture.path().join("tracked"), b"line\r\n").unwrap();
    fixture.git(&["add", ".gitattributes", "tracked"]);
    fixture.git(&["commit", "-q", "-m", "builtin conversion"]);
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200),
    );
    assert_private_output(&fixture, STATUS, b"");
}

#[test]
fn scoped_racy_index_keeps_same_size_rewritten_content_dirty() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "core.trustctime", "false"]);
    let tracked = fixture.path().join("tracked");
    let stamp = SystemTime::UNIX_EPOCH + Duration::new(1_234_567_890, 123_456_700);
    GitIsolationFixture::set_modified(&tracked, stamp);
    fixture.git(&["update-index", "--refresh"]);
    GitIsolationFixture::set_modified(&fixture.path().join(".git/index"), stamp);
    std::fs::write(&tracked, b"BBBB\n").unwrap();
    GitIsolationFixture::set_modified(&tracked, stamp);
    assert_private_output(&fixture, STATUS, b" M tracked\0");
}

#[cfg(unix)]
#[test]
fn scoped_filemode_retains_observed_executable_bit_change() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "core.filemode", "true"]);
    let tracked = fixture.path().join("tracked");
    std::fs::set_permissions(&tracked, std::fs::Permissions::from_mode(0o644)).unwrap();
    fixture.git(&["add", "tracked"]);
    std::fs::set_permissions(&tracked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_private_output(&fixture, STATUS, b" M tracked\0");
}

#[test]
fn scoped_sha1_and_sha256_local_tracking_divergence_matches_real_git() {
    for format in ["sha1", "sha256"] {
        let fixture = GitIsolationFixture::new(format);
        fixture.git(&["config", "remote.origin.url", "."]);
        fixture.git(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ]);
        fixture.git(&["config", "branch.main.remote", "origin"]);
        fixture.git(&["config", "branch.main.merge", "refs/heads/main"]);
        let base = fixture.git(&["rev-parse", "HEAD"]);
        let base = std::str::from_utf8(&base).unwrap().trim();
        let other = fixture.git(&[
            "commit-tree",
            &format!("{base}^{{tree}}"),
            "-p",
            base,
            "-m",
            "other local history",
        ]);
        let other = std::str::from_utf8(&other).unwrap().trim();
        fixture.git(&["update-ref", "refs/remotes/origin/main", other]);
        fixture.git(&["commit", "--allow-empty", "-q", "-m", "local history"]);
        assert_eq!(
            fixture.git(&[
                "rev-list",
                "--left-right",
                "--count",
                "HEAD...refs/remotes/origin/main"
            ]),
            b"1\t1\n"
        );
        let locator = diskgraph_core::QualifiedLocator::from_native_path(fixture.path()).unwrap();
        let before = fixture.metadata();
        let mut session = super::EvidenceProbeSession::new(&ProbeLimits::default()).unwrap();
        let sample = session
            .sample_git_scoped(std::path::Path::new("git"), fixture.path(), &locator)
            .unwrap();
        assert_eq!(sample.ahead_of_upstream, Some(1));
        assert_eq!(sample.behind_upstream, Some(1));
        assert_eq!(sample.dirty_count, 0);
        assert_eq!(
            sample.head.as_ref().unwrap().len(),
            if format == "sha1" { 40 } else { 64 }
        );
        let first = session.resources_for_test();
        let second = session
            .sample_git_scoped(std::path::Path::new("git"), fixture.path(), &locator)
            .unwrap();
        assert_eq!(second.ahead_of_upstream, Some(1));
        let remaining = session.resources_for_test();
        assert!(
            remaining.0 < first.0,
            "second scoped target reset stdout quota"
        );
        assert!(
            remaining.1.unwrap().0 < first.1.unwrap().0,
            "second scoped target reset raw input quota"
        );
        assert!(
            remaining.1.unwrap().1 < first.1.unwrap().1,
            "second scoped target reset input entries"
        );
        fixture.assert_metadata_unchanged(&before);
    }
}

#[test]
fn scoped_private_files_keep_high_precision_mtime_after_writers_close() {
    let fixture = GitIsolationFixture::new("sha1");
    let stamp = SystemTime::UNIX_EPOCH + Duration::new(1_234_567_890, 123_456_700);
    GitIsolationFixture::set_modified(&fixture.path().join("tracked"), stamp);
    GitIsolationFixture::set_modified(&fixture.path().join(".git/index"), stamp);
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = prepare(fixture.path(), &mut probe).unwrap();
    let output = view
        .run(&["rev-parse", "--show-toplevel"], &mut probe)
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    let path =
        super::git_native_path::from_bytes(output.stdout.strip_suffix(b"\n").unwrap()).unwrap();
    let body_time = std::fs::metadata(path.join("tracked"))
        .unwrap()
        .modified()
        .unwrap();
    let index_time = std::fs::metadata(path.join(".git/index"))
        .unwrap()
        .modified()
        .unwrap();
    let checked = view.verify(&mut probe);
    view.complete(checked).unwrap();
    assert_eq!(body_time, stamp);
    assert_eq!(index_time, stamp);
}

#[cfg(unix)]
#[test]
fn scoped_private_executable_mode_and_fixed_status_match_source_observation() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "core.filemode", "true"]);
    let tracked = fixture.path().join("tracked");
    std::fs::set_permissions(&tracked, std::fs::Permissions::from_mode(0o644)).unwrap();
    fixture.git(&["add", "tracked"]);
    for mode in [0o644, 0o755] {
        std::fs::set_permissions(&tracked, std::fs::Permissions::from_mode(mode)).unwrap();
        // 源端 diff 仅作模式差异正控制。私有命令契约限定为采样的固定 status；
        // 实际诊断证实 diff --summary 会改写私有 repo/index，版本防护应继续拒绝。
        let native_diff = fixture.git(&["diff", "--summary"]);
        assert_eq!(
            native_diff,
            if mode == 0o644 {
                b"".as_slice()
            } else {
                b" mode change 100644 => 100755 tracked\n".as_slice()
            }
        );
        let native_status = fixture.git(STATUS);
        assert_eq!(
            native_status,
            if mode == 0o644 {
                b"".as_slice()
            } else {
                b" M tracked\0".as_slice()
            }
        );
        let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
        let mut view = prepare(fixture.path(), &mut probe).unwrap();
        let location = view
            .run(&["rev-parse", "--show-toplevel"], &mut probe)
            .unwrap();
        assert_eq!(location.exit_code, Some(0));
        let path = super::git_native_path::from_bytes(location.stdout.strip_suffix(b"\n").unwrap())
            .unwrap();
        let private_mode = std::fs::metadata(path.join("tracked"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let status = view.run(STATUS, &mut probe);
        let verified = view.verify(&mut probe);
        let status = view.complete(status).unwrap();
        verified.unwrap();
        assert_eq!(private_mode, mode);
        assert_eq!(status.exit_code, Some(0));
        assert_eq!(status.stdout, native_status);
    }
}
