//! Git 固定命令输出的字节级解析，避免格式错误被降级成无 HEAD 或无 upstream。

use super::probe_output::ProbeOutput;

/// 将原探针失败投影为可信 Git 库诊断，保留清理次因。
/// 参数：error 为实际执行错误；返回：不被清理状态覆盖的原命令失败文本。
pub(super) fn execution_error(error: super::probe_failure::ProbeFailure) -> String {
    use super::probe_failure::ProbeFailure;
    match error {
        ProbeFailure::CommandExit { exit_code, stderr } => format!(
            "git exit status {:?}; stderr: {}",
            Some(exit_code),
            if stderr.is_empty() {
                "<empty>".into()
            } else {
                String::from_utf8_lossy(&stderr)
            }
        ),
        ProbeFailure::Cleanup { primary, cleanup } => format!(
            "{}; cleanup also failed: {}",
            execution_error(*primary),
            execution_error(*cleanup)
        ),
        other => other.to_string(),
    }
}

/// 校验 Git 的 SHA-1/SHA-256 对象名，包括 NUL 字段或最多一个末尾 LF 的命令输出。
/// 来源：原生 Rust diskgraph-engine::live_evidence::git_output 与 Git 对象名格式。
/// 参数：bytes 为单个原始对象名字段。返回：保留原字母大小写的完整对象名，或 invalid object id。
pub(super) fn object_id(bytes: &[u8]) -> Result<String, String> {
    let value = without_one_lf(bytes);
    if !matches!(value.len(), 40 | 64)
        || !value.iter().all(u8::is_ascii_hexdigit)
        || value.iter().all(|byte| *byte == b'0')
    {
        return Err("invalid object id".to_owned());
    }
    String::from_utf8(value.to_vec()).map_err(|_| "invalid object id".to_owned())
}

/// 严格解析完整十进制提交数，拒绝空值、符号、空白及 u64 溢出。
/// 来源：原生 Rust diskgraph-engine::live_evidence::git_output 与 git rev-list --count 输出。
/// 参数：bytes 为单行原始计数字节，可有一个末尾 LF。返回：u64 计数或 invalid commit count。
pub(super) fn commit_count(bytes: &[u8]) -> Result<u64, String> {
    let value = without_one_lf(bytes);
    if value.is_empty() {
        return Err("invalid commit count".to_owned());
    }
    value.iter().try_fold(0u64, |count, byte| {
        if !byte.is_ascii_digit() {
            return Err("invalid commit count".to_owned());
        }
        count
            .checked_mul(10)
            .and_then(|count| count.checked_add(u64::from(*byte - b'0')))
            .ok_or_else(|| "invalid commit count".to_owned())
    })
}

/// 按 porcelain v1 -z 的原生 NUL 记录计数，rename/copy 的第二路径不另计。
/// 来源：Git status --porcelain=v1 -z --untracked-files=all 稳定格式。
/// 参数：bytes 为完整 stdout，路径允许非 UTF-8 及换行。返回：文件状态条数或 invalid status record。
pub(super) fn status_count(mut bytes: &[u8]) -> Result<u64, String> {
    let mut count = 0u64;
    while !bytes.is_empty() {
        let (record, rest) = nul_field(bytes)?;
        if record.len() < 4 || record[2] != b' ' || !valid_xy(record[0], record[1]) {
            return Err("invalid status record".to_owned());
        }
        // 第一个路径前固定为 XY 和一个空格；路径本身保持原生字节。
        if record[3..].is_empty() {
            return Err("invalid status record".to_owned());
        }
        bytes = rest;
        if matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C') {
            let (previous_path, rest) = nul_field(bytes)?;
            if previous_path.is_empty() {
                return Err("invalid status record".to_owned());
            }
            bytes = rest;
        }
        count = count
            .checked_add(1)
            .ok_or_else(|| "invalid status record".to_owned())?;
    }
    Ok(count)
}

/// 逐行校验固定 stash OID 输出；空输出表示零条，末条也必须完整终止。
/// 来源：原生 Rust diskgraph-engine::live_evidence::git_output 与 git stash list --format=%H。
/// 参数：bytes 为原始 stash OID 行。返回：stash 数量或 invalid object id 诊断。
pub(super) fn stash_count(bytes: &[u8]) -> Result<u64, String> {
    if bytes.is_empty() {
        return Ok(0);
    }
    let mut count = 0u64;
    for entry in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !entry.ends_with(b"\n") || object_id(entry).is_err() {
            return Err("invalid object id in stash list".to_owned());
        }
        count = count
            .checked_add(1)
            .ok_or_else(|| "invalid stash count".to_owned())?;
    }
    Ok(count)
}

/// 读取一个 UTF-8 引用行，不进行 lossy 转换或任意空白修剪。
/// 来源：原生 Rust diskgraph-engine::live_evidence::git_output 与 Git refname 文本输出。
/// 参数：bytes 为 NUL/EOF 划定的单行字段，可有一个末尾 LF。返回：原文借用或明确格式/UTF-8 错误。
pub(super) fn line(bytes: &[u8]) -> Result<&str, String> {
    let value = without_one_lf(bytes);
    if value.is_empty()
        || value
            .iter()
            .any(|byte| matches!(byte, b'\0' | b'\r' | b'\n'))
    {
        return Err("invalid reference line".to_owned());
    }
    std::str::from_utf8(value).map_err(|_| "reference is not UTF-8".to_owned())
}

/// 仅接受正常退出且 stderr 为空的完整 Git stdout，保留空 stdout 供对应格式解析器判断。
/// 来源：原生 Rust diskgraph-engine::live_evidence::ProbeOutput 执行边界。
/// 参数：output 为已由执行器读至双 EOF 的输出。返回：未经文本转换的 stdout 或退出/stderr 错误。
pub(super) fn successful(output: ProbeOutput) -> Result<Vec<u8>, String> {
    match (output.exit_code, output.stderr.is_empty()) {
        (Some(0), true) => Ok(output.stdout),
        (Some(0), false) => Err(format!(
            "git wrote stderr despite exit 0: {}",
            String::from_utf8_lossy(&output.stderr)
        )),
        (exit, _) => Err(format!(
            "git exit status {exit:?}; stderr: {}",
            if output.stderr.is_empty() {
                "<empty>".into()
            } else {
                String::from_utf8_lossy(&output.stderr)
            }
        )),
    }
}

fn without_one_lf(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}

fn nul_field(bytes: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
        return Err("invalid status record".to_owned());
    };
    Ok((&bytes[..end], &bytes[end + 1..]))
}

fn valid_xy(index: u8, worktree: u8) -> bool {
    // porcelain v1 -z 将子模块小写 m/? 统一为 M，冲突态只有官方定义的七种 XY。
    match (index, worktree) {
        (b'?', b'?')
        | (b'D', b'D')
        | (b'A', b'U')
        | (b'U', b'D')
        | (b'U', b'A')
        | (b'D', b'U')
        | (b'A', b'A')
        | (b'U', b'U')
        | (b'D', b' ') => true,
        (b' ', y) => matches!(y, b'A' | b'M' | b'T' | b'D' | b'R' | b'C'),
        (b'M' | b'T' | b'A' | b'R' | b'C', y) => {
            matches!(y, b' ' | b'M' | b'T' | b'D')
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{commit_count, line, object_id, stash_count, status_count, successful};
    use crate::live_evidence::probe_output::ProbeOutput;

    #[test]
    fn object_ids_accept_both_hash_widths_and_reject_fake_values() {
        assert_eq!(
            object_id(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap()
                .len(),
            40
        );
        assert_eq!(
            object_id(format!("{}\n", "F".repeat(64)).as_bytes())
                .unwrap()
                .len(),
            64
        );
        for value in [
            b"".as_slice(),
            b"abc\n",
            b"0000000000000000000000000000000000000000\n",
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\n",
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaZ",
        ] {
            assert!(object_id(value).unwrap_err().contains("invalid object id"));
        }
    }

    #[test]
    fn commit_counts_accept_only_complete_decimal_u64() {
        assert_eq!(commit_count(b"0\n").unwrap(), 0);
        assert_eq!(commit_count(b"00123").unwrap(), 123);
        assert_eq!(commit_count(b"18446744073709551615\n").unwrap(), u64::MAX);
        for value in [
            b"".as_slice(),
            b"+1",
            b" 1\n",
            b"1\r\n",
            b"1\n2",
            b"18446744073709551616",
        ] {
            assert!(
                commit_count(value)
                    .unwrap_err()
                    .contains("invalid commit count")
            );
        }
    }

    #[test]
    fn status_uses_native_paths_and_one_count_per_rename() {
        assert_eq!(status_count(b"").unwrap(), 0);
        assert_eq!(status_count(b"?? native-\xff\0").unwrap(), 1);
        assert_eq!(
            status_count(b"R  new\nname\0old-\xff\0 M edited\0?? next\0").unwrap(),
            3
        );
        assert_eq!(status_count(b" C copy\0original\0").unwrap(), 1);
    }

    #[test]
    fn status_rejects_bad_codes_and_incomplete_nul_fields() {
        for value in [
            b"?? unterminated".as_slice(),
            b"XY bad\0",
            b"UM invalid-conflict\0",
            b"U  invalid-conflict\0",
            b"M? invalid-worktree\0",
            b"Mm invalid-submodule\0",
            b"DM invalid-deletion\0",
            b"!! ignored-is-not-reported\0",
            b"??xwrong-separator\0",
            b"   unchanged\0",
            b"R  target\0",
            b"R  target\0\0",
            b"?? path\0trailing",
        ] {
            assert!(
                status_count(value)
                    .unwrap_err()
                    .contains("invalid status record")
            );
        }
    }

    #[test]
    fn status_accepts_only_the_seven_unmerged_xy_pairs() {
        for pair in [b"DD", b"AU", b"UD", b"UA", b"DU", b"AA", b"UU"] {
            let record = [pair[0], pair[1], b' ', b'p', 0];
            assert_eq!(status_count(&record).unwrap(), 1);
        }
    }

    #[test]
    fn stash_requires_complete_oid_lines() {
        assert_eq!(stash_count(b"").unwrap(), 0);
        let values = format!("{}\n{}\n", "a".repeat(40), "B".repeat(64));
        assert_eq!(stash_count(values.as_bytes()).unwrap(), 2);
        assert!(stash_count("a".repeat(40).as_bytes()).is_err());
        assert!(stash_count(format!("{}\n\n", "a".repeat(40)).as_bytes()).is_err());
    }

    #[test]
    fn reference_line_preserves_spaces_and_refuses_lossy_bytes() {
        assert_eq!(
            line(" refs/heads/中文 \n".as_bytes()).unwrap(),
            " refs/heads/中文 "
        );
        for value in [b"".as_slice(), b"a\r\n", b"a\nb", b"a\0b", b"bad\xff"] {
            assert!(line(value).is_err());
        }
    }

    #[test]
    fn success_requires_zero_exit_and_empty_stderr() {
        let output = |exit_code, stdout: &[u8], stderr: &[u8]| ProbeOutput {
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
            exit_code,
        };
        assert_eq!(successful(output(Some(0), b"", b"")).unwrap(), b"");
        assert_eq!(
            successful(output(Some(0), b"raw\xff", b"")).unwrap(),
            b"raw\xff"
        );
        assert!(
            successful(output(Some(0), b"valid", b"warning\n"))
                .unwrap_err()
                .contains("stderr")
        );
        assert!(
            successful(output(Some(17), b"", b"failure\n"))
                .unwrap_err()
                .contains("17")
        );
        assert!(
            successful(output(None, b"", b""))
                .unwrap_err()
                .contains("exit")
        );
    }
}
