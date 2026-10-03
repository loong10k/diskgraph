//! EC-04 的 Git for Windows 类型化文件系统缓存回放及真实只读采样回归。

use super::git_config_policy::preserved;
use super::git_configuration::GitConfiguration;
use super::git_isolation_fixture::GitIsolationFixture;

#[test]
fn fscache_boolean_values_are_preserved_and_unknown_core_is_rejected() {
    for value in [
        b"true".as_slice(),
        b"TRUE",
        b"yes",
        b"on",
        b"1",
        b"false",
        b"False",
        b"no",
        b"off",
        b"0",
        b"",
    ] {
        assert_eq!(preserved("core.fscache", value), Ok(true), "{value:?}");
    }
    for value in [b"maybe".as_slice(), b"true; command", b"$(command)"] {
        assert!(
            preserved("core.fscache", value)
                .unwrap_err()
                .contains("unsupported Git boolean: core.fscache"),
            "{value:?}"
        );
    }
    assert!(
        preserved("core.unknowncache", b"true")
            .unwrap_err()
            .contains("unsupported Git core semantics")
    );
    assert_eq!(preserved("core.fsmonitor", b"/external/program"), Ok(false));
}

#[test]
fn fscache_replay_preserves_layer_order_for_real_git() {
    let fixture = GitIsolationFixture::new("sha1");
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().canonicalize().unwrap().join("config");
    let tool_path = super::git_tool_path::from_native(&config_path).unwrap();
    let mut configuration = GitConfiguration::default();
    configuration
        .extend(b"core.fscache\ntrue\0Core.FsCache\nOFF\0core.fscache\0")
        .unwrap();
    let rendered = configuration
        .render(b"/private/attributes", b"/private/excludes")
        .unwrap();
    std::fs::write(&config_path, rendered).unwrap();
    let before = fixture.metadata();
    let values = fixture.git(&[
        "config",
        "--no-includes",
        "--file",
        tool_path.to_str().unwrap(),
        "--type=bool",
        "--get-all",
        "core.fscache",
    ]);
    assert_eq!(values, b"true\nfalse\ntrue\n");
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn native_fscache_preserves_dirty_stash_and_tracking_without_source_writes() {
    for value in ["true", "false"] {
        let fixture = GitIsolationFixture::new("sha1");
        fixture.git(&["config", "core.fscache", value]);
        std::fs::write(fixture.path().join("tracked"), b"stash\n").unwrap();
        fixture.git(&["stash", "push", "-q"]);
        fixture.git(&["config", "remote.origin.url", "."]);
        fixture.git(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ]);
        fixture.git(&["config", "branch.main.remote", "origin"]);
        fixture.git(&["config", "branch.main.merge", "refs/heads/main"]);
        fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        fixture.git(&["commit", "--allow-empty", "-q", "-m", "ahead"]);
        std::fs::write(fixture.path().join("tracked"), b"working\n").unwrap();
        std::fs::write(fixture.path().join("untracked"), b"local\n").unwrap();
        assert_eq!(
            fixture.git(&[
                "--no-optional-locks",
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
            ]),
            b" M tracked\0?? untracked\0"
        );
        let before = fixture.metadata();
        let sample = fixture.sample().unwrap();
        assert_eq!(sample.dirty_count, 2, "core.fscache={value}: {sample:?}");
        assert_eq!(sample.stash_count, 1, "core.fscache={value}: {sample:?}");
        assert_eq!(sample.ahead_of_upstream, Some(1), "{sample:?}");
        assert_eq!(sample.behind_upstream, Some(0), "{sample:?}");
        fixture.assert_metadata_unchanged(&before);

        // 下一次独立采样不能保留前一个 Git 子进程的目录缓存。
        std::fs::write(fixture.path().join("untracked-again"), b"next\n").unwrap();
        let before = fixture.metadata();
        let next = fixture.sample().unwrap();
        assert_eq!(next.dirty_count, 3, "core.fscache={value}: {next:?}");
        fixture.assert_metadata_unchanged(&before);
    }
}
