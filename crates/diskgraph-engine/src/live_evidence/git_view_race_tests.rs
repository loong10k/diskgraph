//! 私有 Git 视图捕获后的配置替换与实时属性宏激活，使用同源原生回调正控制。

use super::ProbeLimits;
use super::git_callback_tests::{command, helper};
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_view::GitView;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const STATUS: &[&str] = &["status", "--porcelain=v1", "-z", "--untracked-files=all"];
const CONTROL_STATUS: &[&str] = &[
    "--no-optional-locks",
    "status",
    "--porcelain=v1",
    "-z",
    "--untracked-files=all",
];

fn private_root(view: &GitView, probe: &mut ProbeBudget) -> PathBuf {
    let output = view
        .run(&["rev-parse", "--absolute-git-dir"], probe)
        .unwrap();
    assert_eq!(output.exit_code, Some(0), "private path: {output:?}");
    let path = super::git_native_path::from_bytes(
        output
            .stdout
            .strip_suffix(b"\n")
            .expect("Git path terminator"),
    )
    .unwrap();
    assert_eq!(path.file_name().unwrap(), "repo");
    path.parent().unwrap().to_path_buf()
}

#[test]
fn replaced_source_config_cannot_execute_after_private_view_capture() {
    let fixture = GitIsolationFixture::new("sha1");
    let program = helper(&fixture);
    let marker = fixture.path().parent().unwrap().join("late-config-marker");
    let original = fixture.metadata();
    // 原生 helper 编译完成后才开始唯一预算；准备、执行、终检和清理不重建预算。
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = GitView::prepare(
        Path::new("git"),
        fixture.path(),
        &mut probe,
        GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
    .unwrap();
    let private = private_root(&view, &mut probe);

    fixture.git(&[
        "config",
        "core.fsmonitor",
        &command(&program, "fsmonitor", &marker),
    ]);
    fixture.git(&["config", "core.fsmonitorHookVersion", "2"]);
    // 夹具命令已经禁用 maintenance，正控制另禁可选锁，避免自行改写源 index。
    let control = fixture.output(CONTROL_STATUS);
    assert!(control.status.success(), "late config control: {control:?}");
    assert_eq!(std::fs::read(&marker).unwrap(), b"fsmonitor");
    std::fs::remove_file(&marker).unwrap();

    let before = fixture.metadata();
    assert_eq!(
        original.iter().map(|item| &item.0).collect::<Vec<_>>(),
        before.iter().map(|item| &item.0).collect::<Vec<_>>(),
        "config replacement/control changed source entry names"
    );
    let changed_files: Vec<_> = original
        .iter()
        .zip(&before)
        .filter(|(old, new)| old != new && !fixture.path().join(".git").join(&old.0).is_dir())
        .map(|(old, _)| old.0.clone())
        .collect();
    assert_eq!(
        changed_files,
        vec![PathBuf::from("config")],
        "the positive control must leave index and other source files unchanged"
    );

    let output = view.run(STATUS, &mut probe).unwrap();
    let verified = view.verify(&mut probe);
    let finished = view.complete(verified);
    probe.check().unwrap();
    assert!(
        !private.exists(),
        "private root survived explicit completion"
    );
    assert!(
        !marker.exists(),
        "private Git read replacement callback config: {output:?}"
    );
    assert_eq!(output.exit_code, Some(0), "private status: {output:?}");
    assert!(output.stdout.is_empty(), "private status: {output:?}");
    let error = finished.expect_err("changed source config must invalidate the view");
    assert!(error.contains("changed"), "terminal rejection: {error}");
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn live_attribute_macro_activating_unused_clean_driver_is_unsupported() {
    assert_live_macro_activation("clean");
}

#[test]
fn live_attribute_macro_activating_unused_process_driver_is_unsupported() {
    assert_live_macro_activation("process");
}

fn assert_live_macro_activation(mode: &str) {
    assert!(matches!(mode, "clean" | "process"));
    let fixture = GitIsolationFixture::new("sha1");
    let program = helper(&fixture);
    let marker = fixture
        .path()
        .parent()
        .unwrap()
        .join("late-attribute-marker");
    fixture.git(&[
        "config",
        &format!("filter.late.{mode}"),
        &command(&program, mode, &marker),
    ]);
    fixture.git(&["config", "filter.late.required", "true"]);
    assert!(!fixture.path().join(".gitattributes").exists());
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = GitView::prepare(
        Path::new("git"),
        fixture.path(),
        &mut probe,
        GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
    .unwrap();
    let private = private_root(&view, &mut probe);
    let unused = view.run(STATUS, &mut probe).unwrap();
    assert_eq!(unused.exit_code, Some(0), "unused driver: {unused:?}");
    assert!(unused.stdout.is_empty(), "unused driver: {unused:?}");
    assert!(!marker.exists(), "unused driver was executed");

    // 实时工作树属性经宏展开激活已捕获但未应用的 driver；内容变化强制转换。
    std::fs::write(
        fixture.path().join(".gitattributes"),
        b"[attr]late_conversion filter=late\ntracked late_conversion\n",
    )
    .unwrap();
    std::fs::write(fixture.path().join("tracked"), b"aaaa\n").unwrap();
    // 长度保持与 index 一致，显式分离 mtime，不能让 size 快路径绕过转换正控制。
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200),
    );
    assert_eq!(
        fixture.git(&["check-attr", "-z", "filter", "--", "tracked"]),
        b"tracked\0filter\0late\0",
        "the real Git attribute macro must select the configured driver"
    );
    let control = fixture.output(CONTROL_STATUS);
    assert_eq!(
        control.status.success(),
        mode == "clean",
        "native {mode} positive control: {control:?}"
    );
    assert_eq!(std::fs::read(&marker).unwrap(), mode.as_bytes());
    if mode == "clean" {
        assert_eq!(
            control.stdout, b"?? .gitattributes\0",
            "uppercase conversion must preserve the tracked index contents"
        );
    }
    std::fs::remove_file(&marker).unwrap();
    let before = fixture.metadata();

    let result = view.run(STATUS, &mut probe);
    let verified = view.verify(&mut probe);
    let finished = view.complete(result);
    probe.check().unwrap();
    assert!(
        !private.exists(),
        "private root survived explicit completion"
    );
    assert!(
        !marker.exists(),
        "private Git executed the newly activated {mode} driver: {finished:?}"
    );
    assert!(
        verified.is_ok(),
        "unchanged captured metadata: {verified:?}"
    );
    let error = finished.expect_err("activated external driver cannot be a normal status");
    assert!(error.to_lowercase().contains("unsupported"), "{error}");
    fixture.assert_metadata_unchanged(&before);
}
