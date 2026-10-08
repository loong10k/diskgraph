//! 私有 Git 视图的提交差异计数，沿用原执行预算和完整 OID。

use super::git_output::{commit_count, successful};
use super::probe_output::ProbeOutput;

/// 参数：head/upstream 为已验证完整OID，run 为原有界命令执行器。
/// 返回：HEAD独有提交数和upstream独有提交数，原命令/格式错误直接传播。
pub(super) fn count(
    head: &str,
    upstream: &str,
    run: &mut impl FnMut(&[&str]) -> Result<ProbeOutput, String>,
) -> Result<(u64, u64), String> {
    let output = successful(run(&[
        "rev-list",
        "--left-right",
        "--count",
        &format!("{head}...{upstream}"),
    ])?)?;
    let pair = output.strip_suffix(b"\n").unwrap_or(&output);
    // 只接受Git固定TAB双字段；禁止单字段解析器容忍字段内的末尾换行。
    if pair.iter().any(|byte| matches!(*byte, b'\n' | b'\r')) {
        return Err("invalid commit count pair".into());
    }
    let separator = pair
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(|| "invalid commit count pair".to_string())?;
    Ok((
        commit_count(&pair[..separator])?,
        commit_count(&pair[separator + 1..])?,
    ))
}

#[cfg(test)]
mod tests {
    use super::count;
    use crate::live_evidence::probe_output::ProbeOutput;

    #[test]
    fn divergence_uses_one_walk_and_keeps_head_then_upstream_direction() {
        for width in [40, 64] {
            let head = "a".repeat(width);
            let upstream = "b".repeat(width);
            let mut commands = Vec::new();
            let actual = count(&head, &upstream, &mut |args| {
                commands.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
                let stdout = if args.contains(&"--left-right") {
                    b"3\t5\n".to_vec()
                } else if args.last() == Some(&format!("{upstream}..{head}").as_str()) {
                    b"3\n".to_vec()
                } else {
                    b"5\n".to_vec()
                };
                Ok(ProbeOutput {
                    stdout,
                    stderr: Vec::new(),
                    exit_code: Some(0),
                })
            })
            .unwrap();
            assert_eq!(actual, (3, 5));
            assert_eq!(commands.len(), 1, "ahead/behind must share one graph walk");
            assert_eq!(
                commands[0],
                [
                    "rev-list",
                    "--left-right",
                    "--count",
                    &format!("{head}...{upstream}")
                ]
            );
        }
    }

    #[test]
    fn incomplete_ambiguous_and_overflowing_pairs_are_rejected() {
        for bytes in [
            b"".as_slice(),
            b"1",
            b"\t2\n",
            b"1\t\n",
            b"1 2\n",
            b"1\t2\t3\n",
            b"+1\t2\n",
            b"1\n\t2\n",
            b"1\t2\n\n",
            b"1\t2\r\n",
            b"1\t\xff\n",
            b"18446744073709551616\t0\n",
            b"0\t18446744073709551616\n",
        ] {
            let result = count(&"a".repeat(40), &"b".repeat(40), &mut |_| {
                Ok(ProbeOutput {
                    stdout: bytes.to_vec(),
                    stderr: Vec::new(),
                    exit_code: Some(0),
                })
            });
            assert!(result.is_err(), "accepted incomplete divergence: {bytes:?}");
        }
    }

    #[test]
    fn complete_pairs_accept_zero_and_u64_boundaries_without_dropping_stderr() {
        for (bytes, expected) in [
            (b"0\t0".as_slice(), (0, 0)),
            (b"18446744073709551615\t0002\n".as_slice(), (u64::MAX, 2)),
            (b"3\t18446744073709551615\n".as_slice(), (3, u64::MAX)),
        ] {
            let result = count(&"a".repeat(40), &"b".repeat(40), &mut |_| {
                Ok(ProbeOutput {
                    stdout: bytes.to_vec(),
                    stderr: Vec::new(),
                    exit_code: Some(0),
                })
            });
            assert_eq!(result.unwrap(), expected);
        }
        let result = count(&"a".repeat(40), &"b".repeat(40), &mut |_| {
            Ok(ProbeOutput {
                stdout: b"0\t0\n".to_vec(),
                stderr: b"original warning".to_vec(),
                exit_code: Some(0),
            })
        });
        assert!(result.unwrap_err().contains("original warning"));
    }

    #[test]
    fn original_execution_failure_is_preserved() {
        let result = count(&"a".repeat(40), &"b".repeat(40), &mut |_| {
            Err("original deadline and cleanup failure".into())
        });
        assert_eq!(result.unwrap_err(), "original deadline and cleanup failure");
    }
}
