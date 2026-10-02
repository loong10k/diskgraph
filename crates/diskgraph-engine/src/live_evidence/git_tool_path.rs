//! 原生路径与 Git 工具路径的词法边界；不重新定位或跟随元数据链接。

use std::path::{Path, PathBuf};

/// 将已验证的原生路径转换为 Git 能保持名称语义的工具路径。
/// 参数：path 为原生文件边界已验证的路径；此函数不替代 no-follow 或身份检查。
/// 返回：Unix 原字节路径，或 Windows 普通本地 drive 路径；不可保真时拒绝。
pub(super) fn from_native(path: &Path) -> Result<PathBuf, String> {
    #[cfg(unix)]
    {
        Ok(path.to_owned())
    }
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        // 普通路径最多 259 个 UTF-16 单元；多取一个单元以拒绝超限输入。
        let units: Vec<u16> = path.as_os_str().encode_wide().take(264).collect();
        Ok(PathBuf::from(OsString::from_wide(&windows_units(&units)?)))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err("unsupported Git tool path platform".into())
    }
}

#[cfg(any(windows, test))]
/// 校验 Windows 原始 UTF-16 工具路径；来源：Windows Win32/Git 路径表示边界。
/// 参数：units 为未经 components() 规范化的完整路径。
/// 返回：可交给 Git 的普通 drive UTF-16，或明确不支持的词法错误。
pub(super) fn windows_units(units: &[u16]) -> Result<Vec<u16>, String> {
    let verbatim = units.starts_with(&[92, 92, 63, 92]);
    let plain = if verbatim { &units[4..] } else { units };
    if plain.len() < 3
        || plain.len() >= 260
        || !matches!(plain[0], 65..=90 | 97..=122)
        || plain[1] != 58
        || !separator(plain[2])
        || (verbatim && plain.contains(&47))
        || char::decode_utf16(plain.iter().copied()).any(|unit| unit.is_err())
    {
        return Err(
            "unsupported Git tool path: absolute local drive representation required".into(),
        );
    }
    if plain.len() > 3 {
        // 单末尾分隔符是普通目录表示，保留它；中间或重复末尾空组件仍拒绝。
        let end = plain.len() - usize::from(plain.last().is_some_and(|unit| separator(*unit)));
        // 检查原始单元，不让 components() 隐去点组件或空组件再进入普通 Win32。
        for name in plain[3..end].split(|unit| separator(*unit)) {
            if name.is_empty()
                || name.len() > 255
                || name == [46]
                || name == [46, 46]
                || matches!(name.last(), Some(32 | 46))
                || name
                    .iter()
                    .any(|unit| matches!(*unit, 0..=31 | 34 | 42 | 58 | 60 | 62 | 63 | 124))
                || reserved_device(name)
            {
                return Err(
                    "unsupported Git tool path: component changes ordinary Win32 identity".into(),
                );
            }
        }
    }
    Ok(plain.to_vec())
}

#[cfg(any(windows, test))]
fn separator(unit: u16) -> bool {
    matches!(unit, 47 | 92)
}

#[cfg(any(windows, test))]
fn reserved_device(name: &[u16]) -> bool {
    let mut end = name
        .iter()
        .position(|unit| *unit == 46)
        .unwrap_or(name.len());
    while end > 0 && name[end - 1] == 32 {
        end -= 1;
    }
    let base: Vec<u16> = name[..end]
        .iter()
        .map(|unit| match unit {
            65..=90 => unit + 32,
            _ => *unit,
        })
        .collect();
    if ["con", "prn", "aux", "nul", "conin$", "conout$"]
        .iter()
        .any(|device| base.iter().copied().eq(device.encode_utf16()))
    {
        return true;
    }
    // Windows 同样把 COM/LPT 后的 ISO-8859-1 上标 1/2/3 识别为设备号。
    base.len() == 4
        && (base.starts_with(&[99, 111, 109]) || base.starts_with(&[108, 112, 116]))
        && matches!(base[3], 49..=57 | 178 | 179 | 185)
}
