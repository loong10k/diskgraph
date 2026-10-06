//! 私有小额度使用与公开入口相同的采样核心；子宿主仅隔离临时目录，不更改全局配额。

use super::ProbeLimits;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_tool_path::from_native;
#[cfg(not(windows))]
use super::git_usage::sample_git_with_resources;
use super::git_view::GitView;
#[cfg(windows)]
use super::native_evidence_test_session::sample_git_with_resources;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;
use std::path::Path;
use std::process::Command;

fn run_isolated(case: &str) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    // 子宿主会把临时根用于 Git 配置路径；工具表示必须仍定位到同一原生目录。
    let tool_root = from_native(&root).unwrap();
    assert_eq!(tool_root.canonicalize().unwrap(), root);
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
        .env("DG_OBJECT_RESOURCE_ROOT", &root)
        .env("TMPDIR", &tool_root)
        .env("TEMP", &tool_root)
        .env("TMP", &tool_root);
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
    #[cfg(windows)]
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("OBJECT_RESOURCE_PATH_CONTROL:{case}")),
        "isolated helper did not exercise the native Git configuration path controls"
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
    let root = std::env::temp_dir();
    let expected_root = std::env::var_os("DG_OBJECT_RESOURCE_ROOT").unwrap();
    assert_eq!(root.canonicalize().unwrap(), Path::new(&expected_root));
    #[cfg(windows)]
    assert_eq!(from_native(&root).unwrap(), root);
    let fixture = GitIsolationFixture::new("sha1");
    #[cfg(windows)]
    assert_windows_config_paths(&fixture, &case);
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

/// 用同一临时文件对照原生及工具路径，确认真实 Git 读取配置的边界。
/// 参数：fixture 为已成功初始化的子宿主仓库，case 为必须回传的执行标识。
/// 返回：无；工具路径必须读出唯一记录，原生路径成功时也须读出同一记录。
#[cfg(windows)]
fn assert_windows_config_paths(fixture: &GitIsolationFixture, case: &str) {
    let config = fixture.path().parent().unwrap().join("config-path-control");
    std::fs::write(&config, format!("[dgpathcontrol]\n marker = {case}\n")).unwrap();
    let native_config = config.canonicalize().unwrap();
    let tool_config = from_native(&native_config).unwrap();
    assert_ne!(native_config, tool_config);
    assert_eq!(tool_config.canonicalize().unwrap(), native_config);

    let args = ["config", "--no-includes", "--null", "--list"];
    let raw = fixture
        .command(&args)
        .env("GIT_CONFIG_GLOBAL", &native_config)
        .output()
        .unwrap();
    let expected = format!("dgpathcontrol.marker\n{case}\0");
    if raw.status.success() {
        // 新 Git 若能读取 verbatim 路径，必须证明读取了同一个文件，不固化旧错误。
        assert_eq!(
            raw.stdout
                .split_inclusive(|byte| *byte == 0)
                .filter(|record| *record == expected.as_bytes())
                .count(),
            1,
            "native config record: {raw:?}"
        );
    } else {
        assert_eq!(raw.status.code(), Some(128), "native config: {raw:?}");
        // init 与 config --list 的配置读取诊断不同；只接受已由 Windows 原生复现的两种错误。
        let stderr = String::from_utf8_lossy(&raw.stderr);
        assert!(
            stderr.contains("unknown error occurred while reading the configuration files")
                || stderr.contains("error processing config file(s)"),
            "native config: {raw:?}"
        );
    }
    let ordinary = fixture
        .command(&args)
        .env("GIT_CONFIG_GLOBAL", &tool_config)
        .output()
        .unwrap();
    assert!(ordinary.status.success(), "tool config: {ordinary:?}");
    assert_eq!(
        ordinary
            .stdout
            .split_inclusive(|byte| *byte == 0)
            .filter(|record| *record == expected.as_bytes())
            .count(),
        1,
        "tool config record: {ordinary:?}"
    );
    println!(
        "OBJECT_RESOURCE_PATH_CONTROL:{case}:raw={:?},ordinary={:?}",
        raw.status.code(),
        ordinary.status.code()
    );
}
