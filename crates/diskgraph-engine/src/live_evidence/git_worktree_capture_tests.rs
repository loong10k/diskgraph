//! D35 工作树捕获真实 RED：旧可信 GitView 的实时读取不是 scoped 隔离。
//! 同步点固定为 prepare 返回后，替换后必须检查成功命令的实际输出，而非最终 Err。

use super::ProbeLimits;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_scoped_fixture::{STATUS, prepare, run_and_complete};
use super::probe_budget::ProbeBudget;

#[test]
fn captured_status_does_not_enumerate_replacement_source_root() {
    let fixture = GitIsolationFixture::new("sha1");
    assert!(fixture.git(STATUS).is_empty());
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let view = prepare(fixture.path(), &mut probe).unwrap();
    let held_source = fixture.path().parent().unwrap().join("captured-original");
    std::fs::rename(fixture.path(), &held_source).unwrap();
    std::fs::create_dir(fixture.path()).unwrap();
    std::fs::write(
        fixture.path().join("tracked"),
        b"foreign replacement bytes\n",
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("outside-only"),
        b"must not be enumerated\n",
    )
    .unwrap();
    let output = run_and_complete(view, &STATUS[1..], &mut probe);
    assert!(
        output.is_empty(),
        "private status read replacement entries: {output:?}"
    );
}

#[test]
fn captured_file_body_is_not_read_from_replacement_source_root() {
    let fixture = GitIsolationFixture::new("sha1");
    let original_oid = fixture.git(&["hash-object", "--no-filters", "tracked"]);
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let view = prepare(fixture.path(), &mut probe).unwrap();
    let held_source = fixture.path().parent().unwrap().join("captured-original");
    std::fs::rename(fixture.path(), &held_source).unwrap();
    std::fs::create_dir(fixture.path()).unwrap();
    std::fs::write(
        fixture.path().join("tracked"),
        b"foreign replacement bytes\n",
    )
    .unwrap();
    let output = run_and_complete(
        view,
        &["hash-object", "--no-filters", "tracked"],
        &mut probe,
    );
    assert_eq!(
        output, original_oid,
        "Git hashed the replacement source body"
    );
}

#[test]
fn captured_nested_directory_and_ignore_rules_do_not_reopen_source() {
    let fixture = GitIsolationFixture::new("sha1");
    std::fs::create_dir(fixture.path().join("nested")).unwrap();
    std::fs::write(fixture.path().join("nested/.gitignore"), b"hidden\n").unwrap();
    std::fs::write(
        fixture.path().join("nested/hidden"),
        b"ignored at capture\n",
    )
    .unwrap();
    fixture.git(&["add", "nested/.gitignore"]);
    fixture.git(&["commit", "-q", "-m", "nested ignore"]);
    assert!(fixture.git(STATUS).is_empty());
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let view = prepare(fixture.path(), &mut probe).unwrap();
    std::fs::rename(
        fixture.path().join("nested"),
        fixture.path().parent().unwrap().join("held-nested"),
    )
    .unwrap();
    std::fs::create_dir(fixture.path().join("nested")).unwrap();
    std::fs::write(fixture.path().join("nested/.gitignore"), b"different\n").unwrap();
    std::fs::write(fixture.path().join("nested/hidden"), b"replacement\n").unwrap();
    let output = run_and_complete(view, &STATUS[1..], &mut probe);
    assert!(
        output.is_empty(),
        "private status used replacement ignore/listing: {output:?}"
    );
}

#[test]
fn captured_attributes_do_not_change_after_source_rewrite() {
    let fixture = GitIsolationFixture::new("sha1");
    std::fs::write(fixture.path().join(".gitattributes"), b"tracked text\n").unwrap();
    fixture.git(&["add", ".gitattributes"]);
    fixture.git(&["commit", "-q", "-m", "attributes"]);
    assert_eq!(
        fixture.git(&["check-attr", "-z", "text", "--", "tracked"]),
        b"tracked\0text\0set\0"
    );
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let view = prepare(fixture.path(), &mut probe).unwrap();
    std::fs::write(fixture.path().join(".gitattributes"), b"tracked -text\n").unwrap();
    let output = run_and_complete(
        view,
        &["check-attr", "-z", "text", "--", "tracked"],
        &mut probe,
    );
    assert_eq!(
        output, b"tracked\0text\0set\0",
        "Git reopened live attributes"
    );
}

#[test]
fn source_file_change_rejects_terminal_result_after_isolated_command() {
    let fixture = GitIsolationFixture::new("sha1");
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = prepare(fixture.path(), &mut probe).unwrap();
    std::fs::write(fixture.path().join("tracked"), b"late source body\n").unwrap();
    let output = view.run(&STATUS[1..], &mut probe).unwrap();
    let terminal = view.verify(&mut probe);
    let terminal = view.complete(terminal);
    assert_eq!(output.exit_code, Some(0));
    assert!(
        output.stdout.is_empty(),
        "isolated status saw late source data"
    );
    assert!(terminal.unwrap_err().contains("changed"));
}

#[test]
fn scoped_session_cumulates_input_budget_and_latches_failure() {
    let fixture = GitIsolationFixture::new("sha1");
    std::fs::write(fixture.path().join("large-untracked"), vec![7u8; 16 * 1024]).unwrap();
    let locator = diskgraph_core::QualifiedLocator::from_native_path(fixture.path()).unwrap();
    let mut session = super::EvidenceProbeSession::new(&ProbeLimits::default()).unwrap();
    session.metadata_limits_for_test(1024, 32_768);
    let first = session
        .sample_git_scoped(std::path::Path::new("git"), fixture.path(), &locator)
        .unwrap_err();
    assert!(first.contains("byte limit"), "{first}");
    std::fs::remove_file(fixture.path().join("large-untracked")).unwrap();
    let second = session
        .sample_git_scoped(
            std::path::Path::new("missing-tool"),
            fixture.path(),
            &locator,
        )
        .unwrap_err();
    assert_eq!(
        first, second,
        "failed session must not replenish input or start another tool"
    );
}
