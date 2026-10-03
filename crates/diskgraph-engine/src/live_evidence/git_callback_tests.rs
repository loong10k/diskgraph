//! 原生 Git 回调的正向控制和私有终态水位；同源夹具在三桌面原生执行。

use super::git_executable::GitExecutable;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_tool_path::from_native;
use super::probe_budget::ProbeBudget;
use super::{ProbeLimits, sample_git_bounded};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

pub(super) fn helper(fixture: &GitIsolationFixture) -> PathBuf {
    let native = fixture
        .path()
        .parent()
        .unwrap()
        .join("callback helper ü.exe");
    let tool = from_native(&native).unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/git_callback_helper.rs");
    let output = Command::new("rustc")
        .args(["--edition=2024", "-O", "-D", "warnings"])
        .arg(source)
        .arg("-o")
        .arg(&tool)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native helper compilation: {output:?}"
    );
    assert_eq!(tool.canonicalize().unwrap(), native.canonicalize().unwrap());
    native.canonicalize().unwrap()
}

fn quote(path: &Path) -> String {
    let tool = from_native(path).unwrap();
    let text = tool.to_str().expect("Unicode temporary callback path");
    #[cfg(windows)]
    let text = text.replace('\\', "/");
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}

pub(super) fn command(program: &Path, mode: &str, marker: &Path) -> String {
    format!("{} {mode} {}", quote(program), quote(marker))
}

fn assert_unsupported(result: Result<super::GitSample, String>) {
    let error = result.expect_err("applied external filter must not be called clean");
    assert!(error.to_lowercase().contains("unsupported"), "{error}");
}

#[test]
fn native_fsmonitor_positive_control_does_not_execute_during_public_sampling() {
    let fixture = GitIsolationFixture::new("sha1");
    let program = helper(&fixture);
    let marker = fixture.path().parent().unwrap().join("fsmonitor-marker");
    fixture.git(&[
        "config",
        "core.fsmonitor",
        &command(&program, "fsmonitor", &marker),
    ]);
    fixture.git(&["config", "core.fsmonitorHookVersion", "2"]);
    let control = fixture.output(&["--no-optional-locks", "status", "--porcelain=v1", "-z"]);
    assert!(control.status.success(), "fsmonitor control: {control:?}");
    assert_eq!(std::fs::read(&marker).unwrap(), b"fsmonitor");
    std::fs::remove_file(&marker).unwrap();
    std::fs::write(fixture.path().join("tracked"), b"BBBB\n").unwrap();
    let before = fixture.metadata();
    let result = fixture.sample();
    assert!(
        !marker.exists(),
        "public sampler executed fsmonitor: {result:?}"
    );
    fixture.assert_metadata_unchanged(&before);
    match result {
        Ok(sample) => assert_eq!(sample.dirty_count, 1, "{sample:?}"),
        Err(error) => assert!(error.to_lowercase().contains("unsupported"), "{error}"),
    }
}

#[test]
fn native_applied_clean_filter_is_unsupported_without_executing_the_callback() {
    let fixture = GitIsolationFixture::new("sha1");
    let program = helper(&fixture);
    let marker = fixture.path().parent().unwrap().join("clean-marker");
    fixture.git(&[
        "config",
        "filter.normalize.clean",
        &command(&program, "clean", &marker),
    ]);
    fixture.git(&["config", "filter.normalize.required", "true"]);
    std::fs::write(
        fixture.path().join(".gitattributes"),
        b"tracked filter=normalize\n",
    )
    .unwrap();
    std::fs::write(fixture.path().join("tracked"), b"lower\n").unwrap();
    fixture.git(&["add", ".gitattributes", "tracked"]);
    fixture.git(&["commit", "-q", "-m", "applied native clean filter"]);
    std::fs::remove_file(&marker).unwrap();
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200),
    );
    let control = fixture.output(&["--no-optional-locks", "status", "--porcelain=v1", "-z"]);
    assert!(
        control.status.success() && control.stdout.is_empty(),
        "clean filter control: {control:?}"
    );
    assert_eq!(std::fs::read(&marker).unwrap(), b"clean");
    std::fs::remove_file(&marker).unwrap();
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_009_843_200),
    );
    let before = fixture.metadata();
    let result = fixture.sample();
    assert!(
        !marker.exists(),
        "public sampler executed clean filter: {result:?}"
    );
    fixture.assert_metadata_unchanged(&before);
    assert_unsupported(result);
}

#[test]
fn native_process_filter_positive_control_is_not_started_by_public_sampling() {
    let fixture = GitIsolationFixture::new("sha1");
    let program = helper(&fixture);
    let marker = fixture.path().parent().unwrap().join("process-marker");
    std::fs::write(
        fixture.path().join(".gitattributes"),
        b"tracked filter=fixture\n",
    )
    .unwrap();
    fixture.git(&["add", ".gitattributes"]);
    fixture.git(&["commit", "-q", "-m", "native process attributes"]);
    fixture.git(&[
        "config",
        "filter.fixture.process",
        &command(&program, "process", &marker),
    ]);
    fixture.git(&["config", "filter.fixture.required", "true"]);
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200),
    );
    let control = fixture.output(&["--no-optional-locks", "status", "--porcelain=v1", "-z"]);
    assert!(
        !control.status.success(),
        "broken process protocol unexpectedly passed: {control:?}"
    );
    assert_eq!(std::fs::read(&marker).unwrap(), b"process");
    std::fs::remove_file(&marker).unwrap();
    let before = fixture.metadata();
    let result = fixture.sample();
    assert!(
        !marker.exists(),
        "public sampler executed process filter: {result:?}"
    );
    fixture.assert_metadata_unchanged(&before);
    assert_unsupported(result);
}

#[test]
fn native_same_size_private_index_rewrite_is_not_a_complete_public_sample() {
    let fixture = GitIsolationFixture::new("sha1");
    let program = helper(&fixture);
    let metrics = fixture
        .path()
        .parent()
        .unwrap()
        .join("private-native-metrics");
    let observed = fixture
        .path()
        .parent()
        .unwrap()
        .join("private-native-observed");
    let git = GitExecutable::resolve(
        Path::new("git"),
        &mut ProbeBudget::new(&ProbeLimits::default()).unwrap(),
    )
    .unwrap();
    // 设置数据只用于本夹具的固定 Git shim，不由生产环境变量继承。
    let settings = format!(
        "{}\n{}\n{}\n",
        from_native(git.path()).unwrap().to_str().unwrap(),
        metrics.to_str().unwrap(),
        observed.to_str().unwrap()
    );
    std::fs::write(program.with_extension("settings"), settings).unwrap();
    let before = fixture.metadata();
    let result = sample_git_bounded(&program, fixture.path(), &ProbeLimits::default());
    assert!(
        observed.exists(),
        "post-status rewrite was not exercised: {result:?}"
    );
    fixture.assert_metadata_unchanged(&before);
    let text = std::fs::read_to_string(&metrics).unwrap();
    let values: Vec<_> = text.lines().collect();
    assert_eq!(values.len(), 8);
    assert_eq!(values[0], values[2], "private length changed");
    assert_eq!(values[1], values[3], "native allocation changed");
    assert_eq!(values[4], values[5], "native full file identity changed");
    assert_ne!(values[6], values[7], "the selected byte was not changed");
    let path = PathBuf::from(std::fs::read_to_string(observed).unwrap());
    assert!(!path.exists(), "private sample cleanup did not complete");
    let error = result.expect_err("same-size private index rewrite became a complete sample");
    assert!(
        error.contains("private Git") && error.contains("changed"),
        "{error}"
    );
}
