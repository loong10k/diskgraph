//! D20 真实攻击正向控制及只读源 metadata 回归；全部限定在临时仓库。

use super::git_isolation_fixture::GitIsolationFixture;
use super::{ProbeLimits, sample_git_bounded};
use sha2::Digest;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

fn assert_unsupported(result: Result<super::GitSample, String>) {
    let error = result.expect_err("unsafe or unsupported repository became a normal sample");
    assert!(error.to_lowercase().contains("unsupported"), "{error}");
}

#[cfg(unix)]
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
#[test]
fn script_writer_fixture() {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("DG_ISOLATION_SCRIPT_PATH") else {
        return;
    };
    let source = std::env::var("DG_ISOLATION_SCRIPT_BYTES").unwrap();
    std::fs::write(&path, source).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
fn fsmonitor_positive_control_cannot_execute_during_sampling() {
    let fixture = GitIsolationFixture::new("sha1");
    let marker = fixture.path().parent().unwrap().join("fsmonitor-marker");
    let script = fixture.script(
        "fsmonitor-hook",
        &format!(
            "#!/bin/sh\nprintf x >> {}\nprintf 'token\\0'\n",
            quote(&marker)
        ),
    );
    fixture.git(&["config", "core.fsmonitor", script.to_str().unwrap()]);
    fixture.git(&["config", "core.fsmonitorHookVersion", "2"]);
    fixture.git(&["status", "--porcelain=v1", "-z"]);
    assert!(
        marker.exists(),
        "real Git did not invoke the fsmonitor control"
    );
    std::fs::remove_file(&marker).unwrap();
    std::fs::write(fixture.path().join("tracked"), b"BBBB\n").unwrap();
    let result = fixture.sample();
    assert!(
        !marker.exists(),
        "sample invoked repository fsmonitor: {result:?}"
    );
    match result {
        Ok(sample) => assert_eq!(sample.dirty_count, 1, "stale fsmonitor cache: {sample:?}"),
        Err(error) => assert!(error.to_lowercase().contains("unsupported"), "{error}"),
    }
}

#[cfg(unix)]
#[test]
fn applied_clean_filter_is_rejected_without_executing_or_guessing_clean() {
    let fixture = GitIsolationFixture::new("sha1");
    let marker = fixture.path().parent().unwrap().join("clean-marker");
    let script = fixture.script(
        "clean-driver",
        &format!(
            "#!/bin/sh\nprintf x >> {}\ntr '[:lower:]' '[:upper:]'\n",
            quote(&marker)
        ),
    );
    fixture.git(&["config", "filter.normalize.clean", &quote(&script)]);
    fixture.git(&["config", "filter.normalize.required", "true"]);
    std::fs::write(
        fixture.path().join(".gitattributes"),
        b"tracked filter=normalize\n",
    )
    .unwrap();
    std::fs::write(fixture.path().join("tracked"), b"lower\n").unwrap();
    fixture.git(&["add", ".gitattributes", "tracked"]);
    fixture.git(&["commit", "-q", "-m", "normalized filter content"]);
    std::fs::remove_file(&marker).unwrap();
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200),
    );
    assert!(fixture.git(&["status", "--porcelain=v1", "-z"]).is_empty());
    assert!(
        marker.exists(),
        "real Git did not execute the applied clean filter"
    );
    std::fs::remove_file(&marker).unwrap();
    // 正向 status 已刷新原缓存；再改变 stat，旧采样不能因缓存命中而绕过哨兵。
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_009_843_200),
    );
    let result = fixture.sample();
    assert!(!marker.exists(), "sample executed clean filter: {result:?}");
    assert_unsupported(result);
}

#[cfg(unix)]
#[test]
fn process_filter_positive_control_is_rejected_before_starting_the_driver() {
    let fixture = GitIsolationFixture::new("sha1");
    let marker = fixture.path().parent().unwrap().join("process-marker");
    let script = fixture.script(
        "process-driver",
        &format!("#!/bin/sh\nprintf x >> {}\nexit 47\n", quote(&marker)),
    );
    std::fs::write(
        fixture.path().join(".gitattributes"),
        b"tracked filter=fixture\n",
    )
    .unwrap();
    fixture.git(&["add", ".gitattributes"]);
    fixture.git(&["commit", "-q", "-m", "attributes"]);
    fixture.git(&["config", "filter.fixture.process", &quote(&script)]);
    fixture.git(&["config", "filter.fixture.required", "true"]);
    GitIsolationFixture::set_modified(
        &fixture.path().join("tracked"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200),
    );
    let control = fixture.output(&["status", "--porcelain=v1", "-z"]);
    assert!(
        !control.status.success(),
        "broken protocol unexpectedly succeeded"
    );
    assert!(
        marker.exists(),
        "real Git did not invoke the process filter"
    );
    std::fs::remove_file(&marker).unwrap();
    let result = fixture.sample();
    assert!(
        !marker.exists(),
        "sample started process filter: {result:?}"
    );
    assert_unsupported(result);
}

#[test]
fn split_index_is_explicitly_unsupported() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["update-index", "--split-index"]);
    assert_unsupported(fixture.sample());
}

#[test]
fn split_index_sampling_does_not_refresh_shared_source_metadata() {
    let fixture = GitIsolationFixture::new("sha1");
    fixture.git(&["update-index", "--split-index"]);
    let shared: Vec<_> = std::fs::read_dir(fixture.path().join(".git"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("sharedindex.")
        })
        .collect();
    assert!(
        !shared.is_empty(),
        "split index fixture has no shared dependency"
    );
    for path in shared {
        GitIsolationFixture::set_modified(
            &path,
            SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200),
        );
    }
    let before = fixture.metadata();
    let _result = fixture.sample();
    fixture.assert_metadata_unchanged(&before);
}

#[test]
fn gitlink_without_gitmodules_is_unsupported_instead_of_clean() {
    let fixture = GitIsolationFixture::new("sha1");
    let oid = String::from_utf8(fixture.git(&["rev-parse", "HEAD"])).unwrap();
    std::fs::create_dir(fixture.path().join("nested")).unwrap();
    fixture.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "160000",
        oid.trim(),
        "nested",
    ]);
    fixture.git(&["commit", "-q", "-m", "gitlink without gitmodules"]);
    assert!(!fixture.path().join(".gitmodules").exists());
    assert!(
        fixture
            .git(&["ls-files", "--stage"])
            .windows(6)
            .any(|part| part == b"160000")
    );
    assert_unsupported(fixture.sample());
}

#[test]
fn unknown_optional_index_cache_is_unsupported_instead_of_clean() {
    let fixture = GitIsolationFixture::new("sha256");
    let index = fixture.path().join(".git/index");
    let mut bytes = std::fs::read(&index).unwrap();
    assert!(bytes.len() > 32);
    bytes.truncate(bytes.len() - 32);
    bytes.extend_from_slice(b"DGXX");
    bytes.extend_from_slice(&0u32.to_be_bytes());
    let checksum = sha2::Sha256::digest(&bytes);
    bytes.extend_from_slice(&checksum);
    std::fs::write(index, bytes).unwrap();
    let control = fixture.output(&["--no-optional-locks", "status", "--porcelain=v1", "-z"]);
    assert!(
        control.status.success(),
        "invalid optional extension fixture: {control:?}"
    );
    assert!(
        control.stdout.is_empty(),
        "fixture is not actually clean: {control:?}"
    );
    assert_unsupported(fixture.sample());
}

#[test]
fn source_metadata_is_unchanged_on_success_budget_failure_and_precancel() {
    let fixture = GitIsolationFixture::new("sha1");
    let before = fixture.metadata();
    fixture.sample().unwrap();
    fixture.assert_metadata_unchanged(&before);
    let limits = ProbeLimits {
        max_output_bytes: 1,
        ..ProbeLimits::default()
    };
    assert!(sample_git_bounded(Path::new("git"), fixture.path(), &limits).is_err());
    fixture.assert_metadata_unchanged(&before);
    let cancelled = ProbeLimits::default();
    cancelled.cancel.store(true, Ordering::Release);
    assert!(sample_git_bounded(Path::new("git"), fixture.path(), &cancelled).is_err());
    fixture.assert_metadata_unchanged(&before);
}
