//! operation_description：既有文件操作职责的原生 Rust 实现。
use std::path::Path;

/// 格式化动作日志说明。
/// 参数：target 为目标路径；identity 为可选对象身份。
/// 返回：目标叶名称及可选身份的展示字符串。
/// A short, redacted description of what a step touched: the target's file name
/// and the identity, never a whole user path.
pub(super) fn describe(target: &Path, identity: Option<&str>) -> String {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unnamed>".into());
    match identity {
        Some(identity) => format!("{name} ({identity})"),
        None => name,
    }
}
