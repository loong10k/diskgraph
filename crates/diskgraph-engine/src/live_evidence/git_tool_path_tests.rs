//! 工具路径词法门禁可在各宿主执行；实际 Git 配对证据仅由 Windows 原生测试提供。

use super::git_tool_path;
use std::path::Path;

fn checked(value: &str) -> Result<String, String> {
    String::from_utf16(&git_tool_path::windows_units(
        &value.encode_utf16().collect::<Vec<_>>(),
    )?)
    .map_err(|error| error.to_string())
}

#[test]
fn local_drive_names_are_preserved_without_lossy_conversion() {
    for path in [
        "C:\\",
        "d:/ordinary/name.txt",
        "C:\\space inside\\中文😀.txt",
        "C:\\e\u{301}\\.git\\config",
    ] {
        assert_eq!(checked(path).unwrap(), path);
    }
}

#[test]
fn only_the_verbatim_drive_prefix_is_removed() {
    assert_eq!(
        checked("\\\\?\\C:\\ordinary\\file.txt").unwrap(),
        "C:\\ordinary\\file.txt"
    );
}

#[test]
fn one_trailing_directory_separator_is_preserved() {
    for path in [
        "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\",
        "d:/ordinary/directory/",
    ] {
        assert_eq!(checked(path).unwrap(), path);
    }
    assert_eq!(
        checked("\\\\?\\C:\\ordinary\\directory\\").unwrap(),
        "C:\\ordinary\\directory\\"
    );
}

#[test]
fn namespaces_and_non_absolute_paths_are_rejected() {
    for path in [
        "",
        "relative",
        "C:relative",
        "\\root-relative",
        "\\\\server\\share\\file",
        "\\\\?\\UNC\\server\\share\\file",
        "\\\\.\\C:\\file",
        "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1\\file",
        "\\\\?\\Volume{1234}\\file",
        "//?/C:/file",
        "\\\\?\\C:/file",
        "1:\\file",
    ] {
        assert!(checked(path).is_err(), "accepted {path:?}");
    }
}

#[test]
fn raw_dot_steps_and_names_changed_by_win32_are_rejected() {
    for path in [
        "C:\\a\\.\\file",
        "C:\\a\\..\\file",
        "C:/a/./file",
        "C:/a/../file",
        "\\\\?\\C:\\a\\..\\file",
        "\\\\?\\C:\\a/file",
        "C:\\a\\file:stream",
        "C:\\a\\file.",
        "C:\\a\\file ",
        "C:\\a\\x?y",
        "C:\\a\\x*y",
        "C:\\a\\x<y",
        "C:\\a\\x>y",
        "C:\\a\\x|y",
        "C:\\a\\x\"y",
        "C:\\a\\x\0y",
        "C:\\a\\x\u{1f}y",
        "C:\\a\\\\file",
        "C:\\a\\\\",
    ] {
        assert!(checked(path).is_err(), "accepted {path:?}");
    }
}

#[test]
fn reserved_devices_are_rejected_even_with_extensions_and_case_variants() {
    for name in [
        "con",
        "PRN.txt",
        "aux",
        "Nul.log",
        "CoN .txt",
        "CONIN$",
        "CONOUT$",
        "COM1",
        "com9.bin",
        "LPT1",
        "lpt9.bin",
        "COM¹",
        "LPT².txt",
        "COM³.bin",
    ] {
        assert!(checked(&format!("C:\\folder\\{name}")).is_err(), "{name}");
    }
    for name in ["com0", "com10", "lpt0", "lpt10", "console", "auxiliary"] {
        assert!(checked(&format!("C:\\folder\\{name}")).is_ok(), "{name}");
    }
}

#[test]
fn ordinary_tool_paths_have_an_explicit_utf16_limit() {
    assert!(checked(&format!("C:\\a\\{}", "x".repeat(254))).is_ok());
    assert!(checked(&format!("C:\\a\\{}", "x".repeat(255))).is_err());
    assert!(checked(&format!("C:\\{}", "x".repeat(256))).is_err());
    assert!(git_tool_path::windows_units(&[67, 58, 92, 0xd800]).is_err());
}

#[cfg(unix)]
#[test]
fn unix_native_bytes_are_not_changed_or_normalized() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(OsStr::from_bytes(b"/exact/\xff/../native.name"));
    assert_eq!(git_tool_path::from_native(path).unwrap(), path);
}

#[cfg(windows)]
#[test]
fn installed_windows_git_pairs_verbatim_and_ordinary_config_paths() {
    use super::ProbeLimits;
    use super::git_command_context::GitCommandContext;
    use super::git_executable::GitExecutable;
    use super::git_system_configuration;
    use super::probe_budget::ProbeBudget;
    use crate::windows_file_state::WindowsFileState;
    use std::ffi::OsStr;
    use std::fs::File;

    let temp = tempfile::Builder::new()
        .prefix("dg git 中文 ")
        .tempdir()
        .unwrap();
    let root = temp.path().canonicalize().unwrap();
    let ordinary = git_tool_path::from_native(&root).unwrap();
    assert!(
        !ordinary
            .as_os_str()
            .to_string_lossy()
            .starts_with("\\\\?\\")
    );
    for name in ["objects", "refs"] {
        std::fs::create_dir_all(root.join("repo").join(name)).unwrap();
    }
    std::fs::write(root.join("repo/HEAD"), b"ref: refs/heads/fixture\n").unwrap();
    let raw_empty = root.join("empty");
    let empty = git_tool_path::from_native(&raw_empty).unwrap();
    std::fs::write(&raw_empty, b"").unwrap();
    let before = WindowsFileState::capture(&File::open(&raw_empty).unwrap()).unwrap();
    assert_eq!(
        before,
        WindowsFileState::capture(&File::open(&empty).unwrap()).unwrap(),
        "ordinary and verbatim names must designate the same native file"
    );

    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let git = GitExecutable::resolve(Path::new("git"), &mut budget).unwrap();
    let printer = [
        OsStr::new("config"),
        OsStr::new("--system"),
        OsStr::new("--edit"),
    ];
    // 同一私有 empty 文件只改变 GLOBAL 表示；不读取或打印宿主配置内容。
    let raw = windows_config_command(git.path(), &ordinary, &raw_empty, &printer, &mut budget);
    eprintln!(
        "verbatim GLOBAL: exit={:?}, stderr={:?}",
        raw.exit_code,
        String::from_utf8_lossy(&raw.stderr)
    );
    let normal = windows_config_command(git.path(), &ordinary, &empty, &printer, &mut budget);
    eprintln!(
        "ordinary GLOBAL: exit={:?}, stderr={:?}",
        normal.exit_code,
        String::from_utf8_lossy(&normal.stderr)
    );
    assert_eq!(normal.exit_code, Some(0), "{normal:?}");
    assert!(normal.stderr.is_empty(), "{normal:?}");
    let host = git_system_configuration::source(&normal.stdout).unwrap();
    let host_before = windows_host_state(&host);

    for path in [&raw_empty, &empty] {
        let args = [
            OsStr::new("config"),
            OsStr::new("--no-includes"),
            OsStr::new("--file"),
            path.as_os_str(),
            OsStr::new("--list"),
            OsStr::new("-z"),
        ];
        let output = windows_config_command(git.path(), &ordinary, &empty, &args, &mut budget);
        eprintln!(
            "config --file {path:?}: exit={:?}, stderr={:?}",
            output.exit_code,
            String::from_utf8_lossy(&output.stderr)
        );
        if path == &empty {
            assert_eq!(output.exit_code, Some(0), "{output:?}");
            assert!(
                output.stdout.is_empty() && output.stderr.is_empty(),
                "{output:?}"
            );
        }
    }
    // 生产上下文必须也使用相同表示；不能由独立正向控制掩盖接线缺失。
    let context = GitCommandContext::new(git.path(), &root, &root, &mut budget).unwrap();
    assert_eq!(
        git_system_configuration::read(&context, &mut budget).unwrap(),
        host
    );
    assert_eq!(
        windows_host_state(&host),
        host_before,
        "the fixed system printer must neither create nor change the host file"
    );
    assert_eq!(
        WindowsFileState::capture(&File::open(&raw_empty).unwrap()).unwrap(),
        before
    );
    assert_eq!(std::fs::read(&empty).unwrap(), b"");
}

#[cfg(windows)]
fn windows_host_state(path: &Path) -> Option<crate::windows_file_state::WindowsFileState> {
    match std::fs::File::open(path) {
        Ok(file) => Some(crate::windows_file_state::WindowsFileState::capture(&file).unwrap()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            panic!("cannot verify host config identity without reading its contents: {error}")
        }
    }
}

#[cfg(windows)]
fn windows_config_command(
    git: &Path,
    directory: &Path,
    global: &Path,
    args: &[&std::ffi::OsStr],
    budget: &mut super::probe_budget::ProbeBudget,
) -> super::probe_output::ProbeOutput {
    use super::probe_execution::run_probe;
    use std::process::Command;
    let mut command = Command::new(git);
    command
        .args(["--no-pager", "--no-lazy-fetch", "--no-optional-locks"])
        .args(args)
        .current_dir(directory)
        .env_clear()
        .env("GIT_DIR", directory.join("repo"))
        .env("GIT_WORK_TREE", directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", global)
        .env("GIT_EDITOR", "printf '%s\\0'")
        .env("GIT_TRACE2", "0")
        .env("GIT_TRACE2_EVENT", "0")
        .env("GIT_TRACE2_PERF", "0");
    if let Some(path) = std::env::var_os("PATH") {
        command.env(
            "PATH",
            std::env::join_paths(std::env::split_paths(&path).filter(|path| path.is_absolute()))
                .unwrap(),
        );
    }
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    run_probe(&mut command, budget).unwrap()
}
