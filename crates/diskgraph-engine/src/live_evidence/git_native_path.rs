//! Git 元数据里的路径保持原生编码，不对元数据叶路径 canonicalize。

use std::path::{Component, Path, PathBuf};

/// 从工具/元数据的路径字节构建原生路径。
/// 参数：bytes 为不含 NUL 的原始路径。返回：Unix 原始字节路径或 Windows UTF-8 路径。
pub(super) fn from_bytes(bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.is_empty() || bytes.contains(&0) {
        return Err("invalid Git native path".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(windows)]
    {
        Ok(PathBuf::from(
            std::str::from_utf8(bytes).map_err(|_| "unsupported Git path encoding")?,
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err("unsupported Git path platform".into())
    }
}

/// 将原生路径作为配置/alternates数据序列化，不进行显示路径转换。
/// 参数：path 为已确认的原生路径。返回：精确字节；不可表示平台路径拒绝。
pub(super) fn bytes(path: &Path) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(path.as_os_str().as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        path.to_str()
            .map(|value| value.as_bytes().to_vec())
            .ok_or_else(|| "unsupported Git path encoding".into())
    }
}

/// 将已验证路径序列化为 Git 可表示的配置数据，保留原生身份路径用于句柄检查。
/// 参数：path 为已捕获的本地路径；返回：Unix 原字节或无歧义的 Windows drive 路径字节。
pub(super) fn tool_bytes(path: &Path) -> Result<Vec<u8>, String> {
    bytes(&super::git_tool_path::from_native(path)?)
}

/// 只消除已验证 base 上的前置父组件，不擦掉未经原生验证的路径组件。
/// 参数：base 为已验证的绝对工作目录，path 为元数据中的相对或绝对路径。
/// 返回：绝对原始路径；Normal 后再 ..、越根及驱动器相对路径明确拒绝。
pub(super) fn resolve(base: &Path, path: &Path) -> Result<PathBuf, String> {
    if !base.is_absolute() {
        return Err("unsupported Git path base".into());
    }
    let mut normal_seen = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => normal_seen = true,
            Component::ParentDir if normal_seen => {
                return Err("unsupported Git path with an unverified parent step".into());
            }
            Component::Prefix(_) if !path.is_absolute() => {
                return Err("unsupported Git drive-relative path".into());
            }
            _ => {}
        }
    }
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return Err("invalid Git metadata parent path".into());
                }
            }
            Component::RootDir | Component::Prefix(_) => out.push(component.as_os_str()),
        }
    }
    if !out.is_absolute() {
        return Err("unsupported Git metadata relative root".into());
    }
    Ok(out)
}

/// 生成 Git alternates 的 C 风格独立路径行，保留冒号、换行和非 UTF-8 字节。
/// 参数：path 为 source ODB 的绝对路径。返回：带引号与换行的 alternates 记录。
#[cfg(test)]
pub(super) fn alternate(path: &Path) -> Result<Vec<u8>, String> {
    let mut out = vec![b'"'];
    for byte in tool_bytes(path)? {
        match byte {
            b'"' | b'\\' => {
                out.push(b'\\');
                out.push(byte);
            }
            0..=31 | 127..=255 => out.extend_from_slice(format!("\\{byte:03o}").as_bytes()),
            _ => out.push(byte),
        }
    }
    out.extend_from_slice(b"\"\n");
    Ok(out)
}
