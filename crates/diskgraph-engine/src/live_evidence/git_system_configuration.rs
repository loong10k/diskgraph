//! 固定参数发现宿主系统配置并解析来源；发现命令仍有单独的原始输入限制。

use super::git_command_context::GitCommandContext;
use super::git_native_path;
use super::git_output::successful;
use super::probe_budget::ProbeBudget;
use std::ffi::OsStr;
use std::path::PathBuf;

/// 在空私有上下文中发现 Git 编译时指定的宿主系统配置。
/// 参数：context 为固定工具与隔离路径，probe 为唯一期限/输出预算。
/// 返回：有界协议输出；此发现步骤尚不是宿主原始配置输入或 RSS 的原生上限。
pub(super) fn read(
    context: &GitCommandContext,
    probe: &mut ProbeBudget,
) -> Result<Vec<u8>, String> {
    let output = context.bootstrap(
        &[
            OsStr::new("config"),
            OsStr::new("--system"),
            OsStr::new("--no-includes"),
            OsStr::new("--show-origin"),
            OsStr::new("--show-scope"),
            OsStr::new("--null"),
            OsStr::new("--get-regexp"),
            OsStr::new("^"),
        ],
        probe,
        true,
        false,
    )?;
    if output.exit_code == Some(1) && output.stdout.is_empty() && output.stderr.is_empty() {
        return Ok(Vec::new());
    }
    successful(output)
}

/// 验证发现输出的固定三字段记录，确定唯一来源与待复核配置数据。
/// 参数：observation 为本次固定发现命令的完整输出。
/// 返回：唯一绝对来源及其 key/value 数据；空配置返回 None，非文件或混合来源拒绝。
pub(super) fn source(observation: &[u8]) -> Result<Option<(PathBuf, Vec<u8>)>, String> {
    if observation.is_empty() {
        return Ok(None);
    }
    if !observation.ends_with(&[0]) {
        return Err("invalid Git system configuration records".into());
    }
    let fields: Vec<&[u8]> = observation[..observation.len() - 1]
        .split(|byte| *byte == 0)
        .collect();
    if !fields.len().is_multiple_of(3) {
        return Err("invalid Git system configuration fields".into());
    }
    let mut origin = None;
    let mut expected = Vec::new();
    for record in fields.as_chunks::<3>().0 {
        if record[0] != b"system" {
            return Err("invalid Git system configuration scope".into());
        }
        let path = git_native_path::from_bytes(
            record[1]
                .strip_prefix(b"file:")
                .ok_or("unsupported Git config origin")?,
        )?;
        if !path.is_absolute() || origin.as_ref().is_some_and(|old| old != &path) {
            return Err("unsupported Git system config origins".into());
        }
        origin = Some(path);
        expected.extend_from_slice(record[2]);
        expected.push(0);
    }
    Ok(Some((origin.ok_or("missing Git config origin")?, expected)))
}
