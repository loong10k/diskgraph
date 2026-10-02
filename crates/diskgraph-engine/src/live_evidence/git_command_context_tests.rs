//! 使用独立宿主检验 PATH 解析，避免并行测试修改进程环境。

use super::git_isolation_fixture::GitIsolationFixture;
use super::{ProbeLimits, sample_git_bounded};
use std::path::Path;
use std::process::Command;

#[test]
fn changing_to_the_worktree_cannot_select_a_repository_git_program() {
    let fixture = GitIsolationFixture::new("sha1");
    let installed = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let marker = fixture.path().join("executed-repository-tool");
    let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
    let script = fixture.script(
        "git",
        &format!(
            "#!/bin/sh\nprintf executed > {}\nexec {} \"$@\"\n",
            quote(&marker),
            quote(&installed)
        ),
    );
    std::fs::rename(script, fixture.path().join("git")).unwrap();
    let path = std::env::join_paths(
        std::iter::once(Path::new("."))
            .chain(std::iter::once(installed.parent().unwrap()))
            .chain([Path::new("/usr/bin"), Path::new("/bin")]),
    )
    .unwrap();
    // 正向控制：相同 PATH 和工作目录确实可以运行仓库中的伪造程序。
    let control = Command::new("git")
        .arg("--version")
        .env("PATH", &path)
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert!(control.status.success(), "{control:?}");
    assert!(marker.exists());
    std::fs::remove_file(&marker).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "live_evidence::git_command_context_tests::path_switch_child",
            "--nocapture",
        ])
        .env("PATH", path)
        .env("DG_GIT_PATH_SWITCH_PROJECT", fixture.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        !marker.exists(),
        "sampling executed the repository's Git program"
    );
}

#[test]
fn path_switch_child() {
    let Some(project) = std::env::var_os("DG_GIT_PATH_SWITCH_PROJECT") else {
        return;
    };
    let result = sample_git_bounded(
        Path::new("git"),
        Path::new(&project),
        &ProbeLimits::default(),
    );
    assert!(result.is_ok(), "{result:?}");
}
