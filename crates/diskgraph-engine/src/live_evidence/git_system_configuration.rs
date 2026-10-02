//! 在空私有配置中仅发现宿主物理路径；首次内容读取必须由原生预算捕获。

use super::git_command_context::GitCommandContext;
use super::git_native_path;
use super::git_output::successful;
use super::probe_budget::ProbeBudget;
use std::ffi::OsStr;
use std::path::PathBuf;

/// 通过固定 printer 获取受信 Git 安装包实际选择的系统配置路径。
/// 参数：context 为固定工具、可信 shell PATH 与私有空配置；probe 为唯一执行预算。
/// 返回：绝对物理路径，包括缺失叶；本接口不解析、创建或读取该宿主文件。
/// 来源：Git 2.46 builtin/config.c 的 system edit、editor.c 的 NULL-buffer 路径接口。
pub(super) fn read(
    context: &GitCommandContext,
    probe: &mut ProbeBudget,
) -> Result<PathBuf, String> {
    let output = context.bootstrap(
        &[
            OsStr::new("config"),
            OsStr::new("--system"),
            OsStr::new("--edit"),
        ],
        probe,
        true,
        false,
    )?;
    source(&successful(output)?)
}

/// 严格解析固定 printer 的单条 NUL 路径记录，不进行 trim 或 UTF-8 替换。
/// 参数：observation 为有界 stdout；必须为单条非空绝对路径及一个末尾 NUL。
/// 返回：原生路径；混合记录、相对路径、超长或不可表示编码明确拒绝。
pub(super) fn source(observation: &[u8]) -> Result<PathBuf, String> {
    let bytes = observation
        .strip_suffix(&[0])
        .filter(|bytes| !bytes.is_empty() && bytes.len() <= 32_768 && !bytes.contains(&0))
        .ok_or("invalid Git system configuration path record")?;
    let path = git_native_path::from_bytes(bytes)?;
    if !path.is_absolute() {
        return Err("unsupported relative Git system configuration path".into());
    }
    Ok(path)
}
