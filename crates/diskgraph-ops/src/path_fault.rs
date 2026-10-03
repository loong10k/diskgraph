//! path_fault：既有文件操作职责的原生 Rust 实现。

/// 作用域下路径逐组件复核的失败原因。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::PathFault`，保留既有语义。
/// Why a path failed pre-execution revalidation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathFault {
    /// A component of the source path is a symbolic link.
    SourceIsLink,
    /// A component of the destination path is a symbolic link.
    TargetIsLink,
    /// A path component vanished between planning and apply.
    ComponentVanished,
}

impl std::fmt::Display for PathFault {
    /// 格式化路径复核错误。
    /// 参数：f 为格式化输出器。
    /// 返回：标准格式化结果。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SourceIsLink => "a source path component is a symbolic link",
            Self::TargetIsLink => "a destination path component is a symbolic link",
            Self::ComponentVanished => "a path component vanished",
        })
    }
}
