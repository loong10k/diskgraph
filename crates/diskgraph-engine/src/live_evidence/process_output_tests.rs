use super::process_output::interpret_process_output;
use super::{ProcessHolder, UsageCoverage};
use std::path::Path;

fn output_for(path: &Path) -> Vec<u8> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut output = b"p42\0cfixture\0n".to_vec();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        output.extend_from_slice(path.as_os_str().as_bytes());
    }
    #[cfg(not(unix))]
    output.extend_from_slice(path.to_str().unwrap().as_bytes());
    output.push(0);
    output
}

#[test]
fn a_multibyte_tag_is_unobservable_without_panicking() {
    let sample = interpret_process_output(b"\xc3\xa9\0", Some(0), &[Path::new("file")], 1);
    assert!(matches!(
        sample.coverage,
        UsageCoverage::Unobservable { .. }
    ));
}

#[test]
fn an_invalid_utf8_tag_is_unobservable_without_panicking() {
    let sample = interpret_process_output(b"\xff\0", Some(0), &[Path::new("file")], 1);
    assert!(matches!(
        sample.coverage,
        UsageCoverage::Unobservable { .. }
    ));
}

#[test]
fn a_successful_empty_probe_cannot_prove_complete_visibility() {
    let sample = interpret_process_output(b"", Some(0), &[Path::new("file")], 1);
    assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    assert!(!sample.verdict().contains("no open handles"));
}

#[test]
fn an_exit_one_empty_probe_cannot_prove_complete_visibility() {
    let sample = interpret_process_output(b"", Some(1), &[Path::new("file")], 1);
    assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    assert!(!sample.verdict().contains("no open handles"));
}

#[test]
fn malformed_pid_or_missing_context_is_unobservable() {
    for output in [
        b"pnot-a-pid\0cfixture\0nfile\0".as_slice(),
        b"p0\0cfixture\0nfile\0",
        b"cfixture\0nfile\0",
        b"p42\0nfile\0",
        b"p42\0c\0nfile\0",
    ] {
        let sample = interpret_process_output(output, Some(0), &[Path::new("file")], 1);
        assert!(
            matches!(sample.coverage, UsageCoverage::Unobservable { .. }),
            "{sample:?}"
        );
    }
}

#[test]
fn an_unterminated_record_is_unobservable() {
    let sample =
        interpret_process_output(b"p42\0cfixture\0nfile", Some(0), &[Path::new("file")], 1);
    assert!(matches!(
        sample.coverage,
        UsageCoverage::Unobservable { .. }
    ));
}

#[test]
fn positive_observations_survive_unverified_coverage() {
    // 解析夹具只使用无歧义 ASCII 字段；真实工具观察由 macOS 夹具另外验证。
    let path = std::path::PathBuf::from(format!("-space fixture {}", uuid::Uuid::new_v4()));
    let output = output_for(&path);
    let sample = interpret_process_output(&output, Some(0), &[&path, &path], 123);
    assert_eq!(sample.sampled_at_unix_ms, 123);
    assert_eq!(
        sample.holders,
        vec![ProcessHolder {
            pid: 42,
            command: "fixture".into()
        }]
    );
    assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    assert!(sample.verdict().contains("1 observed"));
}

#[test]
fn a_literal_deleted_suffix_is_unknown_without_reliable_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file (deleted)");
    std::fs::write(&path, b"data").unwrap();
    let sample = interpret_process_output(&output_for(&path), Some(0), &[&path], 1);
    assert!(sample.holders.is_empty());
    assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    assert!(sample.verdict().contains("path identities are unsupported"));
}

#[test]
fn a_literal_deleted_suffix_cannot_impersonate_a_different_requested_path() {
    let dir = tempfile::tempdir().unwrap();
    let requested = dir.path().join("file");
    let different = dir.path().join("file (deleted)");
    std::fs::write(&requested, b"a").unwrap();
    std::fs::write(&different, b"b").unwrap();
    let sample = interpret_process_output(&output_for(&different), Some(0), &[&requested], 1);
    assert!(sample.holders.is_empty(), "{sample:?}");
    assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    assert!(!sample.verdict().contains("no open handles"));
}

#[test]
fn ordinary_field_separators_and_fd_metadata_remain_compatible() {
    let path = std::path::PathBuf::from(format!("fixture_{}", uuid::Uuid::new_v4()));
    let mut output = b"\np42\0cfixture\0\nf10\0".to_vec();
    let full = output_for(&path);
    output.extend_from_slice(&full[b"p42\0cfixture\0".len()..]);
    output.extend_from_slice(b"\n");
    let sample = interpret_process_output(&output, Some(0), &[&path], 1);
    assert_eq!(sample.holders.len(), 1);
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_stay_unknown_without_lossy_aliases() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let requested = dir.path().join(OsStr::from_bytes(b"file-\xff"));
    let different = dir.path().join(OsStr::from_bytes(b"file-\xfe"));
    // 仅测试原生路径身份；APFS 不接受这些名称，但 Unix OsStr 仍必须保真。
    assert_eq!(requested.to_string_lossy(), different.to_string_lossy());
    let mismatch = interpret_process_output(&output_for(&different), Some(0), &[&requested], 1);
    assert!(mismatch.holders.is_empty(), "{mismatch:?}");
    let exact = interpret_process_output(&output_for(&requested), Some(0), &[&requested], 1);
    assert!(exact.holders.is_empty());
    assert!(matches!(exact.coverage, UsageCoverage::Partial { .. }));
}

#[cfg(windows)]
#[test]
fn a_nonunicode_windows_request_is_explicitly_unobservable() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    let path = std::path::PathBuf::from(OsString::from_wide(&[0xd800]));
    let sample = interpret_process_output(b"", Some(0), &[&path], 1);
    assert!(matches!(
        sample.coverage,
        UsageCoverage::Unobservable { .. }
    ));
}

#[test]
fn a_display_escape_cannot_impersonate_a_native_path() {
    for reported in ["file\\n", "file^A", "file^J", "file\\xe6"] {
        let path = Path::new(reported);
        let output = output_for(path);
        let sample = interpret_process_output(&output, Some(0), &[path], 1);
        assert!(sample.holders.is_empty(), "{sample:?}");
        assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn a_real_lsof_escaped_path_cannot_impersonate_a_different_native_path() {
    use std::fs::File;
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    for (original, alias) in [
        ("line\nfile", "line\\nfile"),
        ("line\u{0001}file", "line^Afile"),
    ] {
        let held_path = dir.path().join(original);
        let different = dir.path().join(alias);
        std::fs::write(&held_path, b"held").unwrap();
        std::fs::write(&different, b"closed").unwrap();
        let _held = File::open(&held_path).unwrap();
        let mut command = Command::new("/usr/sbin/lsof");
        command
            .args(["-w", "-F", "pcn0", "--"])
            .arg(&held_path)
            .env_clear()
            .stdin(Stdio::null());
        if let Some(path_env) = std::env::var_os("PATH") {
            command.env("PATH", path_env);
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        let sample =
            interpret_process_output(&output.stdout, output.status.code(), &[&different], 1);
        assert!(sample.holders.is_empty(), "{sample:?}");
        assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    }
}

#[test]
fn ambiguous_names_do_not_erase_other_positive_observations() {
    let path = std::path::PathBuf::from(format!("fixture_{}", uuid::Uuid::new_v4()));
    let mut output = output_for(&path);
    output.extend_from_slice(b"p43\0cambiguous\0nname\\n\0");
    let sample = interpret_process_output(&output, Some(0), &[&path], 1);
    assert_eq!(
        sample.holders,
        vec![ProcessHolder {
            pid: 42,
            command: "fixture".into()
        }]
    );
    assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    assert!(sample.verdict().contains("path identities are unsupported"));
}
