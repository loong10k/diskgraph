//! 私有 Git metadata/config 视图必须保留普通仓库的真实状态语义。

use super::git_isolation_fixture::GitIsolationFixture;
use std::time::{Duration, SystemTime};

fn configure_tracking(fixture: &GitIsolationFixture) {
    fixture.git(&["config", "remote.origin.url", "."]);
    fixture.git(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
    ]);
    fixture.git(&["config", "branch.main.remote", "origin"]);
    fixture.git(&["config", "branch.main.merge", "refs/heads/main"]);
    fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
}

#[test]
fn ordinary_dirty_stash_and_upstream_match_real_git_without_source_writes() {
    let fixture = GitIsolationFixture::new("sha1");
    for content in [b"first\n".as_slice(), b"second\n"] {
        std::fs::write(fixture.path().join("tracked"), content).unwrap();
        fixture.git(&["stash", "push", "-q"]);
    }
    configure_tracking(&fixture);
    fixture.git(&["commit", "--allow-empty", "-q", "-m", "ahead"]);
    std::fs::write(fixture.path().join("tracked"), b"working\n").unwrap();
    std::fs::write(fixture.path().join("untracked"), b"local\n").unwrap();
    let status = fixture.git(&[
        "--no-optional-locks",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
    ]);
    assert_eq!(status, b" M tracked\0?? untracked\0");
    let before = fixture.metadata();
    let sample = fixture.sample().unwrap();
    assert_eq!(sample.dirty_count, 2, "{sample:?}");
    assert_eq!(sample.stash_count, 2, "{sample:?}");
    assert_eq!(sample.ahead_of_upstream, Some(1), "{sample:?}");
    assert_eq!(sample.behind_upstream, Some(0), "{sample:?}");
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn racy_index_same_size_change_with_restored_mtime_remains_dirty() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "core.trustctime", "false"]);
    let tracked = fixture.path().join("tracked");
    let index = fixture.path().join(".git/index");
    let stamp = SystemTime::UNIX_EPOCH + Duration::new(1_234_567_890, 123_456_700);
    GitIsolationFixture::set_modified(&tracked, stamp);
    fixture.git(&["update-index", "--refresh"]);
    GitIsolationFixture::set_modified(&index, stamp);
    std::fs::write(&tracked, b"BBBB\n").unwrap();
    GitIsolationFixture::set_modified(&tracked, stamp);
    let before = fixture.metadata();
    assert_eq!(
        fixture.git(&["--no-optional-locks", "status", "--porcelain=v1", "-z"]),
        b" M tracked\0"
    );
    let sample = fixture.sample().unwrap();
    assert_eq!(
        sample.dirty_count, 1,
        "fresh private index timestamp hid racy change: {sample:?}"
    );
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn linked_worktree_keeps_its_own_head_index_and_the_common_stash() {
    let fixture = GitIsolationFixture::new("sha1");
    std::fs::write(fixture.path().join("tracked"), b"stash\n").unwrap();
    fixture.git(&["stash", "push", "-q"]);
    let linked = fixture.path().parent().unwrap().join("linked");
    fixture.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "side",
        linked.to_str().unwrap(),
    ]);
    let linked = linked.canonicalize().unwrap();
    std::fs::write(linked.join("tracked"), b"linked\n").unwrap();
    let before = fixture.metadata();
    let sample = super::sample_git_bounded(
        std::path::Path::new("git"),
        &linked,
        &super::ProbeLimits::default(),
    )
    .unwrap();
    assert_eq!(sample.dirty_count, 1, "{sample:?}");
    assert_eq!(sample.stash_count, 1, "{sample:?}");
    assert!(sample.head.is_some());
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn shallow_boundary_is_preserved_in_the_private_view() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["commit", "--allow-empty", "-q", "-m", "next"]);
    let head = fixture.git(&["rev-parse", "HEAD"]);
    std::fs::write(fixture.path().join(".git/shallow"), head).unwrap();
    assert_eq!(
        fixture.git(&["rev-parse", "--is-shallow-repository"]),
        b"true\n"
    );
    assert_eq!(fixture.git(&["rev-list", "--count", "HEAD"]), b"1\n");
    std::fs::write(fixture.path().join("tracked"), b"shallow\n").unwrap();
    let before = fixture.metadata();
    let sample = fixture.sample().unwrap();
    assert_eq!(sample.dirty_count, 1, "{sample:?}");
    assert_eq!(sample.head.as_ref().unwrap().len(), 40);
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn sha256_format_and_tracking_counts_are_preserved_without_skipping_capability() {
    // 本功能要求 Git 2.46+；缺少 SHA256 能力直接失败，不把未运行场景算作通过。
    let fixture = GitIsolationFixture::new("sha256");
    configure_tracking(&fixture);
    fixture.git(&["commit", "--allow-empty", "-q", "-m", "sha256 ahead"]);
    let before = fixture.metadata();
    let sample = fixture.sample().unwrap();
    assert_eq!(sample.head.as_ref().unwrap().len(), 64);
    assert_eq!(sample.dirty_count, 0);
    assert_eq!(sample.ahead_of_upstream, Some(1));
    assert_eq!(sample.behind_upstream, Some(0));
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn builtin_crlf_normalization_is_kept_without_external_filters() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "core.autocrlf", "true"]);
    std::fs::write(fixture.path().join("tracked"), b"line\r\n").unwrap();
    fixture.git(&["add", "tracked"]);
    fixture.git(&["commit", "-q", "-m", "builtin text conversion"]);
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200),
    );
    assert!(
        fixture
            .git(&["--no-optional-locks", "status", "--porcelain=v1", "-z"])
            .is_empty()
    );
    assert_eq!(fixture.sample().unwrap().dirty_count, 0);
}

#[test]
fn info_exclude_patterns_are_preserved_in_untracked_counts() {
    let fixture = GitIsolationFixture::new("sha1");
    std::fs::write(fixture.path().join(".git/info/exclude"), b"*.tmp\n").unwrap();
    std::fs::write(fixture.path().join("ignored.tmp"), b"ignored\n").unwrap();
    std::fs::write(fixture.path().join("visible"), b"visible\n").unwrap();
    assert_eq!(
        fixture.git(&[
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all"
        ]),
        b"?? visible\0"
    );
    assert_eq!(fixture.sample().unwrap().dirty_count, 1);
}

#[test]
fn configured_rename_count_is_not_silently_changed_by_dropping_config() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "status.renames", "false"]);
    std::fs::rename(
        fixture.path().join("tracked"),
        fixture.path().join("renamed"),
    )
    .unwrap();
    fixture.git(&["add", "--all"]);
    assert_eq!(
        fixture.git(&["--no-optional-locks", "status", "--porcelain=v1", "-z"]),
        b"A  renamed\0D  tracked\0"
    );
    assert_eq!(fixture.sample().unwrap().dirty_count, 2);
}

#[cfg(unix)]
#[test]
fn core_filemode_false_does_not_turn_a_clean_file_into_dirty() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "core.filemode", "false"]);
    std::fs::set_permissions(
        fixture.path().join("tracked"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(
        fixture
            .git(&["--no-optional-locks", "status", "--porcelain=v1", "-z"])
            .is_empty()
    );
    assert_eq!(fixture.sample().unwrap().dirty_count, 0);
}

#[test]
fn config_native_case_alias_is_rejected_without_changing_sensitive_volume_semantics() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["config", "status.renames", "false"]);
    std::fs::rename(
        fixture.path().join("tracked"),
        fixture.path().join("renamed"),
    )
    .unwrap();
    fixture.git(&["add", "--all"]);
    let canonical_name = fixture.path().join(".git/config");
    std::fs::rename(&canonical_name, fixture.path().join(".git/CONFIG")).unwrap();
    // 宿主路径解析决定是否为别名，不将大小写敏感卷上的不同名字强行合并。
    let native_alias = canonical_name.exists();
    let real_status = fixture.git(&["--no-optional-locks", "status", "--porcelain=v1", "-z"]);
    let before = fixture.metadata();
    let sampled = fixture.sample();
    if native_alias {
        assert_eq!(real_status, b"A  renamed\0D  tracked\0");
        let error = sampled.expect_err("native CONFIG alias must not become absent configuration");
        assert!(
            error.to_ascii_lowercase().contains("unsupported"),
            "{error}"
        );
    } else {
        assert_eq!(real_status, b"R  renamed\0tracked\0");
        assert_eq!(sampled.unwrap().dirty_count, 1);
    }
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn index_native_case_alias_is_rejected_without_changing_sensitive_volume_semantics() {
    let fixture = GitIsolationFixture::new("sha1");
    let canonical_name = fixture.path().join(".git/index");
    std::fs::rename(&canonical_name, fixture.path().join(".git/INDEX")).unwrap();
    let native_alias = canonical_name.exists();
    let real_status = fixture.git(&[
        "--no-optional-locks",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
    ]);
    let before = fixture.metadata();
    let sampled = fixture.sample();
    if native_alias {
        assert!(real_status.is_empty());
        let error = sampled.expect_err("native INDEX alias must not become an absent index");
        assert!(
            error.to_ascii_lowercase().contains("unsupported"),
            "{error}"
        );
    } else {
        assert_eq!(real_status, b"D  tracked\0?? tracked\0");
        assert_eq!(sampled.unwrap().dirty_count, 2);
    }
    fixture.assert_metadata_unchanged(&before);
}
