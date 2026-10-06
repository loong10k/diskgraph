//! 真实宽工作树验证默认累计管道额度；准备、状态解释与清理均走公开采样入口。

use super::ProbeLimits;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_tool_path::from_native;
#[cfg(windows)]
use super::native_evidence_test_session::sample_git_bounded;
#[cfg(not(windows))]
use super::sample_git_bounded;
use std::path::Path;
use std::process::Command;

#[test]
fn real_wide_status_exhausts_default_output_without_partial_sample_or_private_data() {
    let temp = tempfile::tempdir().unwrap();
    let native_root = temp.path().canonicalize().unwrap();
    let tool_root = from_native(&native_root).unwrap();
    assert_eq!(tool_root.canonicalize().unwrap(), native_root);
    // 独立宿主固定自己的临时根；其他并发采样不会影响私有目录清理断言。
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "live_evidence::git_status_output_tests::wide_status_child_fixture",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("DG_WIDE_STATUS_ROOT", &native_root)
        .env("TMPDIR", &tool_root)
        .env("TEMP", &tool_root)
        .env("TMP", &tool_root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "wide status child: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let completed = stdout
        .lines()
        .find(|line| line.contains("WIDE_STATUS_COMPLETE:"))
        .expect("exact helper did not execute the status and cleanup assertions");
    println!("{completed}");
}

#[test]
#[ignore = "仅由真实宽 status 回归以隔离临时根调用"]
fn wide_status_child_fixture() {
    let expected_root = std::env::var_os("DG_WIDE_STATUS_ROOT").unwrap();
    assert_eq!(
        std::env::temp_dir().canonicalize().unwrap(),
        Path::new(&expected_root)
    );
    let fixture = GitIsolationFixture::new("sha1");
    let before = fixture.metadata();
    let baseline = fixture
        .sample()
        .expect("ordinary preparation must fit the default budget");
    assert_eq!(baseline.dirty_count, 0, "{baseline:?}");
    fixture.assert_metadata_unchanged(&before);
    assert_no_private_data();

    // 普通原生名称不依赖 shell、编码替换或 Git 配置程序；宽度只增加工作树状态记录。
    // 短于常见 Windows 叶路径上限，源 Git 元数据和 bootstrap 输出保持同一份。
    let prefix = "w".repeat(136);
    const FILES: usize = 8192;
    for number in 0..FILES {
        std::fs::write(
            fixture.path().join(format!("{prefix}-{number:04}.txt")),
            b"",
        )
        .unwrap();
    }
    let control = fixture.output(&[
        "--no-optional-locks",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
    ]);
    assert!(control.status.success(), "status control: {control:?}");
    assert!(
        control.stderr.is_empty(),
        "status control stderr: {:?}",
        control.stderr
    );
    let default_limit = ProbeLimits::default().max_output_bytes;
    assert_eq!(default_limit, 1 << 20);
    assert!(
        control.stdout.len() > default_limit,
        "real status output {} did not exceed default {default_limit}",
        control.stdout.len()
    );
    assert_eq!(
        control.stdout.iter().filter(|byte| **byte == 0).count(),
        FILES
    );
    fixture.assert_metadata_unchanged(&before);

    // 默认公开入口必须因实际 status 管道耗尽报错，不能返回已经读到的局部 dirty 数。
    let error = fixture
        .sample()
        .expect_err("wide status returned a partial successful sample");
    assert!(
        error.contains("cumulative output byte limit exceeded"),
        "{error}"
    );
    fixture.assert_metadata_unchanged(&before);
    assert_no_private_data();

    // 仅提高可配置字节额度后，同一真实状态应完整返回；期限和取消仍采用原默认值。
    let expanded = ProbeLimits {
        max_output_bytes: 2 << 20,
        ..ProbeLimits::default()
    };
    let complete = sample_git_bounded(Path::new("git"), fixture.path(), &expanded)
        .expect("the same complete status must fit the explicitly expanded budget");
    assert_eq!(complete.dirty_count, FILES as u64, "{complete:?}");
    fixture.assert_metadata_unchanged(&before);
    assert_no_private_data();
    println!(
        "WIDE_STATUS_COMPLETE: records={FILES} bytes={} limit={default_limit}",
        control.stdout.len()
    );
}

fn assert_no_private_data() {
    for entry in std::fs::read_dir(std::env::temp_dir()).unwrap() {
        assert!(
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("diskgraph-git-"),
            "completed public sample left private data"
        );
    }
}
