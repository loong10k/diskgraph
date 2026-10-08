//! 固定原生命令定位stash，路径保持原始字节，不使用逐行字符串解析。
use super::git_output::{object_id, successful};
use super::probe_output::ProbeOutput;

/// 参数：run为复用原预算的固定Git执行器；返回：初始stash commit OID及原始common路径行。
/// 只接受files后端和完整commit；原执行、stderr及格式错误保留为失败。
pub(super) fn locate(
    run: &mut impl FnMut(&[&str]) -> Result<ProbeOutput, String>,
) -> Result<(String, Vec<u8>), String> {
    let output = successful(run(&[
        "rev-parse",
        "--show-ref-format",
        "--path-format=absolute",
        "--git-common-dir",
        "--verify",
        "--quiet",
        "refs/stash^{commit}",
    ])?)?;
    let remaining = output
        .strip_prefix(b"files\n")
        .ok_or_else(|| "unsupported Git reference backend for complete stash count".to_string())?;
    let frame = remaining
        .strip_suffix(b"\n")
        .ok_or("invalid stash location response")?;
    // 路径自身可包含换行和非UTF-8字节；仅从尾部定位固定ASCII完整OID。
    let separator = frame
        .iter()
        .rposition(|byte| *byte == b'\n')
        .ok_or("invalid stash location response")?;
    let tip = object_id(&frame[separator + 1..])?;
    let common = frame[..separator + 1].to_vec();
    if common.len() <= 1 || common.contains(&0) {
        return Err("invalid Git common directory".into());
    }
    Ok((tip, common))
}

#[cfg(test)]
mod tests {
    use super::locate;
    use crate::live_evidence::probe_output::ProbeOutput;

    #[test]
    fn actual_sha1_and_sha256_location_matches_separate_git_queries() {
        use crate::live_evidence::git_isolation_fixture::GitIsolationFixture;
        for format in ["sha1", "sha256"] {
            let fixture = GitIsolationFixture::new(format);
            fixture.git(&["update-ref", "refs/stash", "HEAD"]);
            #[cfg(target_os = "linux")]
            let name = {
                use std::os::unix::ffi::OsStringExt;
                std::ffi::OsString::from_vec(b"repo line\nbreak\xff".to_vec())
            };
            #[cfg(all(unix, not(target_os = "linux")))]
            let name = std::ffi::OsString::from("repo line\nbreak目录");
            #[cfg(not(unix))]
            let name = std::ffi::OsString::from("repo 目录 spaces");
            let renamed = fixture.path().with_file_name(name);
            std::fs::rename(fixture.path(), &renamed).unwrap();
            let mut run = |args: &[&str]| {
                let output = fixture
                    .command(args)
                    .current_dir(&renamed)
                    .output()
                    .unwrap();
                Ok(ProbeOutput {
                    stdout: output.stdout,
                    stderr: output.stderr,
                    exit_code: output.status.code(),
                })
            };
            let expected_tip = super::object_id(
                &super::successful(
                    run(&["rev-parse", "--verify", "--quiet", "refs/stash^{commit}"]).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
            let expected_common = super::successful(
                run(&["rev-parse", "--path-format=absolute", "--git-common-dir"]).unwrap(),
            )
            .unwrap();
            assert_eq!(locate(&mut run).unwrap(), (expected_tip, expected_common));
        }
    }

    #[test]
    fn fixed_location_queries_use_one_child_and_preserve_raw_path_boundaries() {
        for width in [40, 64] {
            for common in [b"/private/repo\n".as_slice(), b"/private/line\nbreak\xff\n"] {
                let oid = "a".repeat(width);
                let mut calls = Vec::new();
                let result = locate(&mut |args| {
                    calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
                    let stdout = if args.len() > 5 {
                        [b"files\n".as_slice(), common, oid.as_bytes(), b"\n"].concat()
                    } else if args.contains(&"--show-ref-format") {
                        b"files\n".to_vec()
                    } else if args.contains(&"--verify") {
                        format!("{oid}\n").into_bytes()
                    } else {
                        common.to_vec()
                    };
                    Ok(ProbeOutput {
                        stdout,
                        stderr: Vec::new(),
                        exit_code: Some(0),
                    })
                })
                .unwrap();
                assert_eq!(result, (oid, common.to_vec()));
                assert_eq!(
                    calls.len(),
                    1,
                    "stash location must not launch three children"
                );
                assert_eq!(
                    calls[0],
                    [
                        "rev-parse",
                        "--show-ref-format",
                        "--path-format=absolute",
                        "--git-common-dir",
                        "--verify",
                        "--quiet",
                        "refs/stash^{commit}"
                    ]
                );
            }
        }
    }

    #[test]
    fn malformed_frames_and_original_failures_cannot_be_successful_locations() {
        for stdout in [
            b"".as_slice(),
            b"files",
            b"files\n/path",
            b"reftable\n/path\n",
            b"files\n/path\nnot-an-oid\n",
            b"files\n/path\n\n",
        ] {
            assert!(
                locate(&mut |_| Ok(ProbeOutput {
                    stdout: stdout.to_vec(),
                    stderr: Vec::new(),
                    exit_code: Some(0)
                }))
                .is_err()
            );
        }
        assert_eq!(
            locate(&mut |_| Err("original timeout and cleanup".into())).unwrap_err(),
            "original timeout and cleanup"
        );
        assert!(
            locate(&mut |_| Ok(ProbeOutput {
                stdout: b"files\n/path\n".to_vec(),
                stderr: b"original corruption".to_vec(),
                exit_code: Some(128)
            }))
            .unwrap_err()
            .contains("original corruption")
        );
    }
}
