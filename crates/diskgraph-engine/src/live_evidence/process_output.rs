//! 保守解释占用探针：仅匹配可确认的显示路径，未确认身份保持未知。

use super::{ProcessHolder, UsageCoverage, UsageSample};
use std::collections::HashSet;
use std::path::Path;

/// 按字段字节解析占用输出，保持未经验证的权限及进程启动身份为 partial。
/// 参数：output 为完整 stdout，exit 为退出码，paths 为查询路径，sampled_at_unix_ms 为采样时间。
/// 返回：正向占用观察与覆盖诊断；异常字段为不可观察样本。
pub(super) fn interpret_process_output(
    output: &[u8],
    exit: Option<i32>,
    paths: &[&Path],
    sampled_at_unix_ms: u64,
) -> UsageSample {
    // lsof 的空结果可退出 1；退出码不能证明权限范围或 PID 启动上下文。
    let parsed = if exit == Some(0) || (exit == Some(1) && output.is_empty()) {
        parse_holders(output, paths)
    } else {
        Err(format!("the handle probe exited with {exit:?}"))
    };
    let (coverage, holders) = match parsed {
        Ok((holders, identity_unknown)) => {
            let mut reason =
                "system visibility, probe warnings and process start identities were not verified"
                    .to_owned();
            if identity_unknown {
                reason.push_str("; ambiguous displayed path identities are unsupported");
            }
            (UsageCoverage::Partial { reason }, holders)
        }
        Err(reason) => (UsageCoverage::Unobservable { reason }, Vec::new()),
    };
    UsageSample {
        sampled_at_unix_ms,
        coverage,
        holders,
    }
}

fn parse_holders(output: &[u8], paths: &[&Path]) -> Result<(Vec<ProcessHolder>, bool), String> {
    // 字段必须以 NUL 结束；字段集之间允许 lsof 使用换行分隔。
    if output
        .iter()
        .rev()
        .find(|byte| **byte != b'\n')
        .is_some_and(|byte| *byte != 0)
    {
        return Err("the handle probe returned an unterminated record".into());
    }
    // 原生键集合避免显示转换碰撞，也避免每条记录遍历全部查询路径。
    let requested: HashSet<Vec<u8>> = paths
        .iter()
        .map(|path| {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            native_path_bytes(&canonical)
        })
        .collect::<Result<_, _>>()?;
    let mut holders = Vec::new();
    let mut identity_unknown = false;
    let mut current_pid = None;
    let mut current_command = None;
    for mut record in output.split(|byte| *byte == 0) {
        while record.first() == Some(&b'\n') {
            record = &record[1..];
        }
        let Some((&tag, value)) = record.split_first() else {
            continue;
        };
        // 标签是协议字节，不对 UTF-8 字符的任意字节位置切片。
        match tag {
            b'p' => {
                let pid = std::str::from_utf8(value)
                    .ok()
                    .filter(|_| !value.is_empty() && value.iter().all(u8::is_ascii_digit))
                    .and_then(|text| text.parse::<u32>().ok())
                    .filter(|pid| *pid != 0)
                    .ok_or("the handle probe returned an invalid PID")?;
                current_pid = Some(pid);
                current_command = None;
            }
            b'c' => {
                if current_pid.is_none() || value.is_empty() {
                    return Err("the handle probe returned an incomplete process context".into());
                }
                current_command = Some(
                    std::str::from_utf8(value)
                        .map_err(|_| "the handle probe returned an invalid command encoding")?
                        .to_owned(),
                );
            }
            b'n' => {
                let (Some(pid), Some(command_name)) = (current_pid, current_command.as_ref())
                else {
                    return Err("the handle probe returned an incomplete process context".into());
                };
                if value.is_empty() {
                    return Err("the handle probe returned an empty path".into());
                }
                // NUL 只限定字段边界；lsof 仍可能转义名称，显示文本不能冒充原生身份。
                // 没有已验证的可逆协议时不猜测解码，也不把删除标记当原始名称。
                if value.iter().any(|byte| {
                    !byte.is_ascii() || byte.is_ascii_control() || matches!(*byte, b'\\' | b'^')
                }) || value.ends_with(b" (deleted)")
                {
                    identity_unknown = true;
                    continue;
                }
                if requested.contains(value) {
                    holders.push(ProcessHolder {
                        pid,
                        command: command_name.clone(),
                    });
                }
            }
            // lsof 可能附带文件描述符等 ASCII 字段，保留兼容但不推导新事实。
            tag if tag.is_ascii_alphabetic() => {}
            _ => return Err("the handle probe returned an invalid field tag".into()),
        }
    }
    holders.sort();
    holders.dedup();
    Ok((holders, identity_unknown))
}

fn native_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(path.as_os_str().as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        path.to_str()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| "the handle probe cannot represent this native path losslessly".into())
    }
}
