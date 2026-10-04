use diskgraph_core::{BusinessError, DiskNode, NodeKind, WindowsFileObservation};

/// 已授权 revision 目录的原生身份；来源：Unix dev/inode 与 D31 Windows 完整观测。
/// 不比较目录修改时间，正常仓库内容变化不会被误判为节点替换。
#[derive(Clone)]
pub(crate) struct GitIndexedDirectory {
    #[cfg(unix)]
    pub(super) unix: (u64, u64),
    #[cfg(windows)]
    pub(super) windows: (u64, [u8; 16], i64),
}

impl GitIndexedDirectory {
    /// 从持久节点构造预期身份。参数：node 为同 revision 节点，windows 为完整原生观测。
    /// 返回：精确身份或需重新索引的 Unsupported；不从显示路径或截断 Windows ID 猜测。
    pub(crate) fn from_node(
        node: &DiskNode,
        windows: Option<&WindowsFileObservation>,
    ) -> Result<Self, BusinessError> {
        if node.kind != NodeKind::Directory || node.read_error {
            return Err(BusinessError::Unsupported);
        }
        #[cfg(unix)]
        {
            let _ = windows;
            let identity = node
                .file_identity
                .as_ref()
                .ok_or(BusinessError::Unsupported)?;
            let volume = identity
                .volume_id
                .parse()
                .map_err(|_| BusinessError::Unsupported)?;
            Ok(Self {
                unix: (volume, identity.file_id),
            })
        }
        #[cfg(windows)]
        {
            let observed = windows.ok_or(BusinessError::Unsupported)?;
            Ok(Self {
                windows: windows_identity(observed)?,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = windows;
            Err(BusinessError::Unsupported)
        }
    }
}

/// 参数：实际捕获的 Windows 完整观测；返回：完整目录身份或明确不支持。
/// 来源：D31 完整捕获与 WindowsFileState 目录安全属性；EOF 对齐不作为目录身份条件。
/// Unverified 只说明树尺寸未对齐；仍必须有完整捕获，并由 held-source 复验完整身份。
#[cfg(any(windows, test))]
pub(super) fn windows_identity(
    observed: &WindowsFileObservation,
) -> Result<(u64, [u8; 16], i64), BusinessError> {
    observed
        .validate()
        .map_err(|_| BusinessError::Unsupported)?;
    if !observed.directory
        || observed.delete_pending
        // 与 WindowsFileState::validate(true) 相同的 Win32 原生属性含义：
        // REPARSE_POINT / OFFLINE / RECALL_ON_OPEN / RECALL_ON_DATA_ACCESS。
        || observed.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
    {
        return Err(BusinessError::Unsupported);
    }
    Ok((observed.volume, observed.file_id, observed.creation_time))
}
