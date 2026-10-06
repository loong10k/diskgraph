//! 由独立原生宿主检验同名工具正控制和工作目录切换，不修改并行测试环境。

use super::ProbeLimits;
use super::git_executable::GitExecutable;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_tool_path::from_native;
#[cfg(windows)]
use super::native_evidence_test_session::sample_git_bounded;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;
#[cfg(not(windows))]
use super::sample_git_bounded;
use std::path::{Path, PathBuf};
use std::process::Command;

const CHILD: &str = "live_evidence::git_executable_native_tests::native_shadow_child_fixture";
const COMPLETE: &str = "NATIVE_GIT_SHADOW_COMPLETE";

#[test]
fn native_repository_git_shadow_is_not_selected_after_worktree_switch() {
    let fixture = GitIsolationFixture::new("sha1");
    let installed = GitExecutable::resolve(
        Path::new("git"),
        &mut ProbeBudget::new(&ProbeLimits::default()).unwrap(),
    )
    .unwrap();
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    let native = fixture.path().join(name);
    let program = from_native(&native).unwrap();
    // 编译器可以生成同级副产物；全部留在工作树外，只将可执行夹具复制到仓库。
    let build = fixture.path().parent().unwrap().join("shadow-build");
    std::fs::create_dir(&build).unwrap();
    let compiled_program = from_native(&build.join(name)).unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/git_callback_helper.rs");
    let compiled = Command::new("rustc")
        .args(["--edition=2024", "-O", "-D", "warnings"])
        .arg(source)
        .arg("-o")
        .arg(&compiled_program)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "native shadow compile: {compiled:?}"
    );
    std::fs::copy(&compiled_program, &native).unwrap();
    assert_eq!(
        program.canonicalize().unwrap(),
        native.canonicalize().unwrap()
    );

    // 固定绝对 Git，沿用夹具的隔离环境；原始 NUL 输出必须精确确认唯一工作树变更。
    let prepared = fixture.command(&[
        "--no-optional-locks",
        "--no-lazy-fetch",
        "-c",
        "core.fsmonitor=false",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
    ]);
    let status = Command::new(installed.path())
        .env_clear()
        .envs(
            prepared
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .args(prepared.get_args())
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert!(status.status.success(), "native shadow status: {status:?}");
    assert_eq!(
        status.stdout,
        format!("?? {name}\0").into_bytes(),
        "native shadow worktree entries: {status:?}"
    );

    let installed_parent = from_native(installed.path().parent().unwrap()).unwrap();
    let host = std::env::var_os("PATH").unwrap();
    let absolute_host: Vec<_> = std::env::split_paths(&host)
        .filter(|path| path.is_absolute())
        .filter_map(|path| from_native(&path).ok())
        .collect();
    let path = std::env::join_paths(
        std::iter::once(PathBuf::from("."))
            .chain(std::iter::once(installed_parent))
            .chain(absolute_host),
    )
    .unwrap();
    let before = fixture.metadata();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", CHILD, "--nocapture"])
        .env("PATH", path)
        .env("DG_NATIVE_SHADOW_PROJECT", fixture.path())
        .env("DG_NATIVE_SHADOW_CALLER", fixture.path().parent().unwrap())
        .current_dir(fixture.path().parent().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "native shadow child: {output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("1 passed; 0 failed"),
        "child did not run: {stdout}"
    );
    assert!(
        stdout.contains(COMPLETE),
        "child did not complete: {stdout}"
    );
    fixture.assert_metadata_unchanged(&before);
    assert!(!native.with_extension("marker").exists());
}

#[test]
#[ignore = "parent runs this exact isolated native PATH/cwd fixture"]
fn native_shadow_child_fixture() {
    let project = PathBuf::from(std::env::var_os("DG_NATIVE_SHADOW_PROJECT").unwrap());
    let caller = PathBuf::from(std::env::var_os("DG_NATIVE_SHADOW_CALLER").unwrap());
    assert_eq!(project.parent().unwrap(), caller);
    let marker = project.join("git.marker");
    assert!(!marker.exists());
    std::env::set_current_dir(&project).unwrap();
    // 正控制必须通过真实原生名称搜索运行仓库中的同名程序，不能仅检查文件存在。
    let control = Command::new("git").arg("--version").output().unwrap();
    assert!(
        control.status.success(),
        "native shadow control: {control:?}"
    );
    assert_eq!(std::fs::read(&marker).unwrap(), b"shadow-executed");
    std::fs::remove_file(&marker).unwrap();

    // 固定宿主 PATH 中只有当前项为相对路径；生产解析须固定可信绝对程序后再切工作树。
    std::env::set_current_dir(&caller).unwrap();
    let result = sample_git_bounded(Path::new("git"), &project, &ProbeLimits::default());
    assert!(
        !marker.exists(),
        "public sampler selected repository program: {result:?}"
    );
    let sample = result.expect("native repository shadow must not affect the sample");
    // 未跟踪的同名可执行夹具仍是一个实际工作树变更，不能被省略成 clean。
    assert_eq!(sample.dirty_count, 1, "{sample:?}");
    println!("{COMPLETE}");
}
