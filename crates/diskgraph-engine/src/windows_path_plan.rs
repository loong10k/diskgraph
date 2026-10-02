use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};

use diskgraph_core::BusinessError;

use crate::EngineError;

/// Windows 原生内容路径规划；来源：Windows drive 路径及 NT 相对组件语义。
/// 只做词法检查，不通过客户端路径的 canonicalize 跟随 junction 或链接。
pub(crate) struct WindowsPathPlan {
    pub(crate) drive_root: PathBuf,
    pub(crate) components: Vec<OsString>,
}

impl WindowsPathPlan {
    /// 检查注册根和请求路径，返回 drive 根及完整逐组件计划；拒绝 ADS 和逃逸。
    pub(crate) fn new(root: &Path, path: &Path) -> Result<Self, EngineError> {
        let (root_drive, root_parts) = parts(root)?;
        let (path_drive, path_parts) = parts(path)?;
        if root_drive != path_drive || !path_parts.starts_with(&root_parts) {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        if path_parts.len() <= root_parts.len() {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
        Ok(Self {
            drive_root: PathBuf::from(format!("{}:\\", char::from(root_drive))),
            components: path_parts,
        })
    }
}

fn parts(path: &Path) -> Result<(u8, Vec<OsString>), EngineError> {
    // 有界 UTF-16 与句柄数；禁止 components() 会规范化掉的中间点组件。
    let wide: Vec<u16> = path.as_os_str().encode_wide().take(32768).collect();
    if wide.len() >= 32768
        || wide.contains(&0)
        || wide
            .split(|unit| *unit == u16::from(b'\\') || *unit == u16::from(b'/'))
            .any(|part| part == [46] || part == [46, 46])
    {
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    }
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    };
    let drive = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) if drive.is_ascii_alphabetic() => {
            drive.to_ascii_uppercase()
        }
        _ => return Err(EngineError::Business(BusinessError::Unsupported)),
    };
    if components.next() != Some(Component::RootDir) {
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    }
    let mut names = Vec::new();
    for component in components {
        let Component::Normal(name) = component else {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        };
        if names.len() >= 1024 || name.encode_wide().any(|unit| unit == u16::from(b':')) {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
        names.push(name.to_os_string());
    }
    Ok((drive, names))
}
