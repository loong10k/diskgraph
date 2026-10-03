//! 私有小额度使用与公开入口相同的采样核心；子宿主仅隔离临时目录，不更改全局配额。

use super::ProbeLimits;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_usage::sample_git_with_resources;
use super::git_view::GitView;
use super::probe_budget::ProbeBudget;
use std::path::Path;
use std::process::Command;

fn run_isolated(case: &str) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "live_evidence::git_object_resources_tests::resource_child_fixture",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("DG_OBJECT_RESOURCE_CASE", case)
        .env("TMPDIR", &root)
        .env("TEMP", &root)
        .env("TMP", &root);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{case}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("OBJECT_RESOURCE_COMPLETE:{case}")),
        "isolated helper did not execute its exact case"
    );
}

#[test]
fn sampler_preparation_resource_failure_removes_private_data() {
    run_isolated("prepare");
}

#[test]
fn sampler_object_copy_input_failure_removes_private_data() {
    run_isolated("copy-input");
}

#[test]
fn sampler_object_copy_allocation_failure_removes_private_data() {
    run_isolated("copy-allocation");
}

#[test]
fn sampler_terminal_input_failure_removes_private_data() {
    run_isolated("terminal");
}

#[test]
#[ignore = "仅由四个真实子宿主回归以隔离临时根调用"]
fn resource_child_fixture() {
    let case = std::env::var("DG_OBJECT_RESOURCE_CASE").unwrap();
    let fixture = GitIsolationFixture::new("sha1");
    // 真实不易压缩 blob，确保额度用于对象文件，而不是少量 bootstrap 配置。
    let mut value = 7_u64;
    let bytes: Vec<_> = (0..262_144)
        .map(|_| {
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            value as u8
        })
        .collect();
    std::fs::write(fixture.path().join("payload"), bytes).unwrap();
    fixture.git(&["hash-object", "-w", "payload"]);
    let (input, allocation) = match case.as_str() {
        "prepare" => (4, 128 << 20),
        "copy-input" => (32 << 10, 128 << 20),
        "copy-allocation" => (16 << 20, 128 << 10),
        "terminal" => (400 << 10, 128 << 20),
        _ => panic!("unknown isolated resource case"),
    };
    if case == "terminal" {
        let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
        let mut prepared = GitView::prepare(
            Path::new("git"),
            fixture.path(),
            &mut probe,
            GitMetadataBudget::new(input, 32_768).unwrap(),
            allocation,
            0,
        )
        .unwrap();
        prepared.complete(Ok(())).unwrap();
    }
    let before = fixture.metadata();
    let result = sample_git_with_resources(
        Path::new("git"),
        fixture.path(),
        &ProbeLimits::default(),
        GitMetadataBudget::new(input, 32_768).unwrap(),
        allocation,
        0,
    );
    fixture.assert_metadata_unchanged(&before);
    for entry in std::fs::read_dir(std::env::temp_dir()).unwrap() {
        assert!(
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("diskgraph-git-"),
            "resource failure left private sample data"
        );
    }
    let error = result.expect_err("shared sampler did not charge source objects");
    let expected = if case == "copy-allocation" {
        "Git object copy: private Git allocation"
    } else {
        "git metadata byte limit exceeded"
    };
    assert!(error.contains(expected), "{case}: {error}");
    println!("OBJECT_RESOURCE_COMPLETE:{case}");
}
