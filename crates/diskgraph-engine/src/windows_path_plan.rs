use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};

use diskgraph_core::BusinessError;

use crate::EngineError;

/// Windows 原生内容路径规划；来源：Windows drive 路径及 NT 相对组件语义。
/// 只做词法检查，不通过客户端路径的 canonicalize 跟随 junction 或链接。
pub(crate) struct WindowsPathPlan {
    drive: u8,
    pub(crate) drive_root: PathBuf,
    pub(crate) components: Vec<OsString>,
}

#[cfg(test)]
thread_local! {
    static PARSE_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl WindowsPathPlan {
    /// 读取本测试线程实际词法解析次数。参数：无；返回：累计次数，不参与生产预算。
    #[cfg(test)]
    pub(crate) fn parse_work_for_tests() -> usize {
        PARSE_WORK.with(std::cell::Cell::get)
    }

    /// 规划注册根本身；允许本地 drive 根，不跟随任何路径链接。
    /// 参数：root 为完整本地 drive 路径，组件保留原生名称。
    /// 返回：drive 根及到注册根的组件，或词法/namespace 错误。
    pub(crate) fn for_root(root: &Path) -> Result<Self, EngineError> {
        let (drive, components) = parts(root)?;
        Ok(Self {
            drive,
            drive_root: PathBuf::from(format!("{}:\\", char::from(drive))),
            components,
        })
    }

    /// 取得注册根下的精确相对组件；相等路径返回空组件用于根属性读取。
    /// 参数：root/path 为同一 drive 的原生绝对路径，名称不做大小写猜测。
    /// 返回：安全的相对组件；越界、点步、ADS 或不支持 namespace 明确拒绝。
    pub(crate) fn relative_to_root(root: &Path, path: &Path) -> Result<Vec<OsString>, EngineError> {
        Self::for_root(root)?.relative_path(path)
    }

    /// 使用固定根的词法计划检查请求路径；不缓存文件身份或授权。
    /// 参数：path 为原生绝对路径；返回：根下相对组件，越界或非法路径拒绝。
    pub(crate) fn relative_path(&self, path: &Path) -> Result<Vec<OsString>, EngineError> {
        let (path_drive, path_parts) = parts(path)?;
        if self.drive != path_drive || !path_parts.starts_with(&self.components) {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        Ok(path_parts[self.components.len()..].to_vec())
    }

    /// 检查注册根和请求路径，返回 drive 根及完整逐组件计划；拒绝 ADS 和逃逸。
    /// 参数：root/path 为注册根和精确 Windows 本地 drive 路径。
    /// 返回：drive 根及逐组件计划；ADS、逃逸或不支持路径返回错误。
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
            drive: root_drive,
            drive_root: PathBuf::from(format!("{}:\\", char::from(root_drive))),
            components: path_parts,
        })
    }
}

fn parts(path: &Path) -> Result<(u8, Vec<OsString>), EngineError> {
    #[cfg(test)]
    PARSE_WORK.with(|count| count.set(count.get() + 1));
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
