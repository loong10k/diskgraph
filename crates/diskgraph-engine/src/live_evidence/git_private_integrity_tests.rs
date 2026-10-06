//! 私有元数据与清理对象身份的真实回归；所有操作限定在独占夹具。

use super::ProbeLimits;
use super::git_private_directory::GitPrivateDirectory;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;

#[cfg(unix)]
#[test]
fn private_index_mutation_fixture() {
    use std::io::{Seek, SeekFrom, Write};
    use std::os::unix::fs::MetadataExt;
    let Some(path) = std::env::var_os("DG_PRIVATE_INDEX_MUTATE") else {
        return;
    };
    let before = std::fs::metadata(&path).unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(7)).unwrap();
    file.write_all(&[255]).unwrap();
    drop(file);
    let after = std::fs::metadata(&path).unwrap();
    std::fs::write(
        std::env::var_os("DG_PRIVATE_INDEX_METRICS").unwrap(),
        serde_json::to_vec(&[before.len(), before.blocks(), after.len(), after.blocks()]).unwrap(),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn same_allocation_private_index_mutation_cannot_be_a_successful_public_sample() {
    use super::git_isolation_fixture::GitIsolationFixture;
    let fixture = GitIsolationFixture::new("sha1");
    let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let quote =
        |path: &std::path::Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\"'\"'"));
    let observed = fixture
        .path()
        .parent()
        .unwrap()
        .join("private-index-observed");
    let metrics = fixture
        .path()
        .parent()
        .unwrap()
        .join("private-index-metrics");
    let tool = fixture.script(
        "private-index-mutator",
        &format!(
            "#!/bin/sh\ncase \" $* \" in\n*' status '*)\n {} \"$@\"\n code=$?\n DG_PRIVATE_INDEX_MUTATE=\"$GIT_INDEX_FILE\" DG_PRIVATE_INDEX_METRICS={} {} --exact live_evidence::git_private_integrity_tests::private_index_mutation_fixture --quiet > /dev/null || exit 65\n printf '%s' \"$GIT_INDEX_FILE\" > {}\n exit \"$code\";;\n*) exec {} \"$@\";;\nesac\n",
            quote(&git), quote(&metrics), quote(&std::env::current_exe().unwrap()),
            quote(&observed), quote(&git),
        ),
    );
    let before = fixture.metadata();
    let result = super::sample_git_bounded(&tool, fixture.path(), &ProbeLimits::default());
    assert!(observed.exists(), "mutation was not exercised: {result:?}");
    fixture.assert_metadata_unchanged(&before);
    // 原生测量私有文件本身，证明不是长度增长或分配超额触发失败。
    let values: [u64; 4] = serde_json::from_slice(&std::fs::read(metrics).unwrap()).unwrap();
    assert_eq!(values[0], values[2], "private file length changed");
    assert_eq!(values[1], values[3], "private allocation changed");
    let private = std::path::PathBuf::from(std::fs::read_to_string(observed).unwrap());
    assert!(!private.exists(), "private cleanup did not finish");
    let error = result.expect_err("changed private index was reported as a complete sample");
    assert!(
        error.contains("private Git") && error.contains("changed"),
        "{error}"
    );
}

#[test]
fn cleanup_refuses_a_foreign_directory_that_replaced_the_registered_root() {
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(1 << 20, 0, &mut probe).unwrap();
    let original = directory.path().to_owned();
    let temp = tempfile::tempdir().unwrap();
    let moved = temp.path().join("moved-owner");
    std::fs::rename(&original, &moved).unwrap();
    std::fs::create_dir(&original).unwrap();
    let marker = original.join("foreign-data");
    std::fs::write(&marker, b"must survive cleanup").unwrap();
    let error = directory
        .complete::<()>(Err("fixture primary failure".into()))
        .unwrap_err();
    let preserved = marker.exists();
    // 先收回本测试创建的替身，失败断言也不能把它留在系统临时目录。
    if original.exists() {
        std::fs::remove_dir_all(&original).unwrap();
    }
    // 恢复原名称后，让同一真实 owner 显式清理；不在登记之外直接删除它。
    std::fs::rename(&moved, &original).unwrap();
    directory.complete(Ok(())).unwrap();
    assert!(!original.exists());
    assert!(preserved, "cleanup deleted a foreign replacement: {error}");
    assert!(error.contains("fixture primary failure"));
    assert!(
        error.contains("cleanup") && error.contains("identity"),
        "{error}"
    );
}
