//! 在第一次命令前固定工具路径，后续工作目录不能改变程序选择。

use super::probe_budget::ProbeBudget;
use std::path::{Path, PathBuf};

/// 本次采样唯一的绝对受信 Git 程序路径。
/// 来源：原生 Rust DiskGraph D20 工具执行边界，无 Java 对应实现。
pub(super) struct GitExecutable {
    path: PathBuf,
}

impl GitExecutable {
    /// 从显式路径或仅包含绝对目录的 PATH 候选中解析一次程序。
    /// 参数：tool 为受信工具标识，probe 为整次期限与取消预算。
    /// 返回：固定绝对程序；不搜索空、当前或相对 PATH 项。
    pub(super) fn resolve(tool: &Path, probe: &mut ProbeBudget) -> Result<Self, String> {
        probe.check().map_err(|error| error.to_string())?;
        let candidate = if tool.is_absolute() {
            tool.to_owned()
        } else if tool.components().count() != 1 {
            #[cfg(windows)]
            return Err("unsupported relative Git executable path".into());
            #[cfg(not(windows))]
            {
                // 显式相对路径只相对于调用宿主解析一次，不能相对于后续工作树。
                std::env::current_dir()
                    .map_err(|error| format!("Git executable directory: {error}"))?
                    .join(tool)
            }
        } else {
            let name = executable_name(tool)?;
            let path = std::env::var_os("PATH").ok_or_else(|| {
                probe.fail(super::probe_failure::ProbeFailure::Io(
                    "Git executable requires PATH".into(),
                ));
                "Git executable requires PATH"
            })?;
            let mut selected = None;
            for directory in std::env::split_paths(&path).filter(|path| path.is_absolute()) {
                probe.check().map_err(|error| error.to_string())?;
                let candidate = directory.join(&name);
                if executable_file(&candidate) {
                    selected = Some(candidate);
                    break;
                }
            }
            selected.ok_or_else(|| {
                probe.fail(super::probe_failure::ProbeFailure::Io(
                    "Git executable absent".into(),
                ));
                "Git executable absent from absolute PATH directories"
            })?
        };
        let path = candidate.canonicalize().map_err(|error| {
            let message = format!("Git executable path: {error}");
            probe.fail(super::probe_failure::ProbeFailure::io(
                "Git executable path",
                error,
            ));
            message
        })?;
        if !executable_file(&path) {
            return Err("unsupported Git executable file".into());
        }
        #[cfg(windows)]
        if !path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
        {
            return Err("unsupported Git executable: native .exe required".into());
        }
        probe.check().map_err(|error| error.to_string())?;
        Ok(Self { path })
    }

    /// 借用本次采样固定的绝对程序路径。
    /// 参数：无；返回：已完成解析的受信路径。
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

fn executable_name(tool: &Path) -> Result<std::ffi::OsString, String> {
    if tool.as_os_str().is_empty() || tool.file_name() != Some(tool.as_os_str()) {
        return Err("invalid Git executable name".into());
    }
    #[cfg(windows)]
    {
        let mut name = tool.as_os_str().to_owned();
        match tool.extension() {
            None => name.push(".exe"),
            Some(value)
                if value
                    .to_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case("exe")) => {}
            Some(_) => return Err("unsupported Git executable extension".into()),
        }
        Ok(name)
    }
    #[cfg(not(windows))]
    Ok(tool.as_os_str().to_owned())
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}
