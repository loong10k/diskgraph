//! 固定宿主路径发现先于任何原始配置解析，全部文件位于隔离夹具。

use super::ProbeLimits;
use super::git_command_context::GitCommandContext;
#[cfg(unix)]
use super::git_metadata_budget::GitMetadataBudget;
#[cfg(unix)]
use super::git_metadata_file::GitMetadataFile;
use super::git_system_configuration;
use super::probe_budget::ProbeBudget;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn bootstrap(tool: &Path, probe: &mut ProbeBudget) -> (tempfile::TempDir, GitCommandContext) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for name in ["objects", "refs"] {
        std::fs::create_dir_all(root.join("repo").join(name)).unwrap();
    }
    std::fs::write(root.join("repo/HEAD"), b"ref: refs/heads/fixture\n").unwrap();
    std::fs::write(root.join("empty"), b"").unwrap();
    let context = GitCommandContext::new(tool, &root, &root, probe).unwrap();
    (temp, context)
}

#[cfg(unix)]
fn installed_git() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap()
}

#[cfg(unix)]
fn wrapper(root: &Path, source: &Path) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    fn quote(path: &Path) -> Vec<u8> {
        let mut bytes = vec![b'\''];
        for byte in path.as_os_str().as_bytes() {
            if *byte == b'\'' {
                bytes.extend_from_slice(b"'\\''");
            } else {
                bytes.push(*byte);
            }
        }
        bytes.push(b'\'');
        bytes
    }
    let tool = root.join("git-system-fixture");
    let mut script = b"#!/bin/sh\nexport GIT_CONFIG_SYSTEM=".to_vec();
    script.extend_from_slice(&quote(source));
    script.extend_from_slice(b"\nexec ");
    script.extend_from_slice(&quote(&installed_git()));
    script.extend_from_slice(b" \"$@\"\n");
    write_script(&tool, &script);
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    tool
}

#[cfg(unix)]
fn write_script(path: &Path, bytes: &[u8]) {
    use std::os::unix::ffi::OsStringExt;
    // 独立 writer 退出后才执行脚本，防止并行 fork 暂时继承写句柄而引起 ETXTBSY。
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "live_evidence::git_system_configuration_tests::script_writer_child",
            "--quiet",
            "--test-threads=1",
        ])
        .env_clear()
        .env("DG_SYSTEM_SCRIPT_PATH", path)
        .env(
            "DG_SYSTEM_SCRIPT_BYTES",
            std::ffi::OsString::from_vec(bytes.to_vec()),
        )
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
}

#[test]
#[cfg(unix)]
fn script_writer_child() {
    use std::os::unix::ffi::OsStrExt;
    let Some(path) = std::env::var_os("DG_SYSTEM_SCRIPT_PATH") else {
        return;
    };
    let bytes = std::env::var_os("DG_SYSTEM_SCRIPT_BYTES").unwrap();
    std::fs::write(path, bytes.as_bytes()).unwrap();
}

#[cfg(unix)]
fn fifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
}

#[test]
#[cfg(unix)]
fn oversized_host_file_is_rejected_by_native_capture_before_git_parses_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("large-config");
    let mut content = b"[user]\nname = ".to_vec();
    content.extend(std::iter::repeat_n(b'x', 128 << 10));
    content.push(b'\n');
    std::fs::write(&source, &content).unwrap();
    let before = std::fs::metadata(&source).unwrap().modified().unwrap();
    let limits = ProbeLimits {
        max_output_bytes: 512,
        ..ProbeLimits::default()
    };
    let mut probe = ProbeBudget::new(&limits).unwrap();
    let (_bootstrap, context) = bootstrap(&wrapper(&root, &source), &mut probe);
    let result = git_system_configuration::read(&context, &mut probe);
    assert!(
        result.is_ok(),
        "discovery parsed oversized raw configuration: {result:?}"
    );
    assert_eq!(result.unwrap(), source);
    let error = GitMetadataFile::capture(
        &source,
        &mut GitMetadataBudget::new(64, 16).unwrap(),
        &mut probe,
    )
    .err()
    .unwrap();
    assert!(error.contains("byte limit"), "{error}");
    assert_eq!(std::fs::read(&source).unwrap(), content);
    assert_eq!(
        std::fs::metadata(&source).unwrap().modified().unwrap(),
        before
    );
}

#[test]
#[cfg(unix)]
fn fifo_host_configuration_is_only_located_then_native_capture_rejects_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("config-fifo");
    fifo(&source);
    let limits = ProbeLimits {
        timeout: Duration::from_secs(2),
        ..ProbeLimits::default()
    };
    let mut probe = ProbeBudget::new(&limits).unwrap();
    let (_bootstrap, context) = bootstrap(&wrapper(&root, &source), &mut probe);
    let result = git_system_configuration::read(&context, &mut probe);
    assert!(result.is_ok(), "discovery opened host FIFO: {result:?}");
    assert_eq!(result.unwrap(), source);
    assert!(
        GitMetadataFile::capture(&source, &mut GitMetadataBudget::default(), &mut probe,).is_err()
    );
}

#[test]
#[cfg(unix)]
fn malformed_host_config_and_included_fifo_are_not_parsed_during_discovery() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let included = root.join("included-fifo");
    fifo(&included);
    let source = root.join("config-with-include");
    let content = format!("[include]\npath = {}\n[invalid\n", included.display());
    std::fs::write(&source, &content).unwrap();
    let before = std::fs::metadata(&source).unwrap().modified().unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let (_bootstrap, context) = bootstrap(&wrapper(&root, &source), &mut probe);
    let result = git_system_configuration::read(&context, &mut probe);
    assert!(
        result.is_ok(),
        "discovery parsed source config/include: {result:?}"
    );
    assert_eq!(result.unwrap(), source);
    let captured =
        GitMetadataFile::capture(&source, &mut GitMetadataBudget::default(), &mut probe).unwrap();
    assert_eq!(captured.bytes().unwrap(), content.as_bytes());
    assert_eq!(
        std::fs::metadata(&source).unwrap().modified().unwrap(),
        before
    );
}

#[test]
fn path_record_is_single_absolute_native_path_without_display_normalization() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join(" space\nname ");
    let mut bytes = super::git_native_path::bytes(&path).unwrap();
    bytes.push(0);
    assert_eq!(git_system_configuration::source(&bytes).unwrap(), path);
    for invalid in [
        Vec::new(),
        vec![0],
        b"relative\0".to_vec(),
        b"/one\0/two\0".to_vec(),
        b"/one\0\0".to_vec(),
        b"/no-terminator".to_vec(),
        vec![b'x'; 32_770],
    ] {
        assert!(git_system_configuration::source(&invalid).is_err());
    }
    let mut oversized = super::git_native_path::bytes(&path).unwrap();
    oversized.resize(32_769, b'x');
    oversized.push(0);
    assert!(git_system_configuration::source(&oversized).is_err());
}

#[test]
fn actual_installed_git_discovers_one_path_without_creating_the_host_file() {
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let (_temp, context) = bootstrap(Path::new("git"), &mut probe);
    let path: PathBuf = git_system_configuration::read(&context, &mut probe).unwrap();
    assert!(path.is_absolute());
    let before = std::fs::symlink_metadata(&path)
        .ok()
        .map(|metadata| (metadata.len(), metadata.modified().unwrap()));
    assert_eq!(
        git_system_configuration::read(&context, &mut probe).unwrap(),
        path
    );
    let after = std::fs::symlink_metadata(&path)
        .ok()
        .map(|metadata| (metadata.len(), metadata.modified().unwrap()));
    assert_eq!(after, before);
}

#[test]
fn discovery_budget_and_cancellation_are_not_reset() {
    let limits = ProbeLimits {
        max_output_bytes: 1,
        ..ProbeLimits::default()
    };
    let mut probe = ProbeBudget::new(&limits).unwrap();
    let (_temp, context) = bootstrap(Path::new("git"), &mut probe);
    assert!(
        git_system_configuration::read(&context, &mut probe)
            .unwrap_err()
            .contains("byte limit")
    );
    assert!(
        git_system_configuration::read(&context, &mut probe)
            .unwrap_err()
            .contains("byte limit")
    );
    let limits = ProbeLimits::default();
    let mut probe = ProbeBudget::new(&limits).unwrap();
    let (_temp, context) = bootstrap(Path::new("git"), &mut probe);
    limits.cancel.store(true, Ordering::Release);
    assert!(
        git_system_configuration::read(&context, &mut probe)
            .unwrap_err()
            .contains("cancel")
    );
    let mut expired = ProbeBudget::new(&ProbeLimits {
        timeout: Duration::ZERO,
        ..ProbeLimits::default()
    })
    .unwrap();
    assert!(
        git_system_configuration::read(&context, &mut expired)
            .unwrap_err()
            .contains("deadline")
    );
}

#[test]
fn host_configuration_flag_cannot_reopen_the_original_parser() {
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let (_temp, context) = bootstrap(Path::new("git"), &mut probe);
    for args in [
        vec![
            OsStr::new("config"),
            OsStr::new("--system"),
            OsStr::new("--list"),
        ],
        vec![OsStr::new("var"), OsStr::new("GIT_CONFIG_SYSTEM")],
        vec![
            OsStr::new("config"),
            OsStr::new("--system"),
            OsStr::new("--edit"),
            OsStr::new("extra"),
        ],
    ] {
        let error = context
            .bootstrap(&args, &mut probe, true, false)
            .err()
            .unwrap();
        assert!(error.contains("unsupported Git host"), "{error}");
    }
    assert!(
        context
            .bootstrap(
                &[
                    OsStr::new("config"),
                    OsStr::new("--system"),
                    OsStr::new("--edit")
                ],
                &mut probe,
                true,
                true,
            )
            .is_err()
    );
}

#[test]
#[cfg(unix)]
fn missing_and_shell_metacharacter_names_are_returned_exactly_without_creation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let marker = root.join("shell-executed");
    let names = [
        "missing".to_owned(),
        " space ' quote\nname ".to_owned(),
        "dollar$(printf unsafe)".to_owned(),
        "`touch marker`;echo unsafe".to_owned(),
    ];
    for name in names {
        let source = root.join(name);
        let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
        let (_bootstrap, context) = bootstrap(&wrapper(&root, &source), &mut probe);
        assert_eq!(
            git_system_configuration::read(&context, &mut probe).unwrap(),
            source
        );
        assert!(!source.exists());
    }
    assert!(!marker.exists());
    assert!(!root.join("marker").exists());
}

#[test]
#[cfg(unix)]
fn physical_host_alias_is_rediscovered_after_redirection() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let a = root.join("a");
    let b = root.join("b");
    std::fs::write(&a, b"[core]\nfilemode = true\n").unwrap();
    std::fs::write(&b, b"[core]\nfilemode = false\n").unwrap();
    let alias = root.join("host-alias");
    std::os::unix::fs::symlink(&a, &alias).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let (_bootstrap, context) = bootstrap(&wrapper(&root, &alias), &mut probe);
    let before = git_system_configuration::read(&context, &mut probe).unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&b, &alias).unwrap();
    let after = git_system_configuration::read(&context, &mut probe).unwrap();
    assert_eq!(before, a);
    assert_eq!(after, b);
    assert_ne!(before, after);
}

#[test]
#[cfg(target_os = "linux")]
fn unix_non_utf8_host_paths_preserve_native_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join(std::ffi::OsString::from_vec(b"config-\xff".to_vec()));
    std::fs::write(&source, b"[core]\nfilemode = true\n").unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let (_bootstrap, context) = bootstrap(&wrapper(&root, &source), &mut probe);
    assert_eq!(
        git_system_configuration::read(&context, &mut probe).unwrap(),
        source
    );
}

#[test]
#[cfg(unix)]
fn shell_path_is_filtered_once_and_injected_startup_environment_is_cleared() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let tool = root.join("environment-fixture");
    write_script(&tool, b"#!/bin/sh\nif test -n \"${BASH_ENV+x}${ENV+x}${LD_PRELOAD+x}${GIT_TRACE+x}\"; then exit 17; fi\nprintf '%s\\0' \"$PATH\"\n");
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    let expected = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    let inherited = std::env::join_paths([
        Path::new("."),
        Path::new(""),
        Path::new("relative"),
        Path::new("/usr/bin"),
        Path::new("/bin"),
    ])
    .unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "live_evidence::git_system_configuration_tests::shell_environment_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .env("PATH", inherited)
        .env("DG_SYSTEM_ENV_TOOL", &tool)
        .env("DG_SYSTEM_ENV_PATH", expected)
        .env("BASH_ENV", root.join("must-not-read"))
        .env("ENV", root.join("must-not-read"))
        .env("LD_PRELOAD", root.join("must-not-load"))
        .env("GIT_TRACE", root.join("must-not-write"))
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(!root.join("must-not-write").exists());
}

#[test]
#[cfg(unix)]
fn shell_environment_child() {
    use std::os::unix::ffi::OsStrExt;
    let Some(tool) = std::env::var_os("DG_SYSTEM_ENV_TOOL") else {
        return;
    };
    let expected = std::env::var_os("DG_SYSTEM_ENV_PATH").unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let (_temp, context) = bootstrap(Path::new(&tool), &mut probe);
    // 只在隔离、单测试子宿主中改环境；并行主测试和其他代理的宿主不受影响。
    unsafe {
        std::env::set_var("PATH", ".");
    }
    let output = context
        .bootstrap(
            &[
                OsStr::new("config"),
                OsStr::new("--system"),
                OsStr::new("--edit"),
            ],
            &mut probe,
            true,
            false,
        )
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    let mut expected = expected.as_bytes().to_vec();
    expected.push(0);
    assert_eq!(output.stdout, expected);
    assert!(output.stderr.is_empty());
}
