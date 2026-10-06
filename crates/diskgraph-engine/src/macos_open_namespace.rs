use crate::macos_filesystem_state::MacosFilesystemState;
use std::fs::File;

/// 原生逐组件打开得到的祖先句柄、镜像句柄及身份，不克隆或改变原句柄所有权。
/// 来源：原生 Rust PF-06 安装租约；无 Java 对等对象，沿用原 tuple 返回契约。
pub(super) type MacosOpenNamespace = (
    Vec<(File, MacosFilesystemState)>,
    File,
    MacosFilesystemState,
);
