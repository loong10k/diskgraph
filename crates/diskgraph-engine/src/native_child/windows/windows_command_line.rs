//! 显式环境和 CRT 参数引号规则；CreateProcessW 不搜索当前工作目录。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::super::ChildError;

const MAX_INPUT_UNITS: usize = 32_767;

/// 已验证的 CreateProcessW 输入。来源：Win32 CreateProcessW 与 Windows CRT argv、Unicode 环境块契约。
pub(super) struct WindowsCommandLine {
    application: Vec<u16>,
    arguments: Vec<u16>,
    environment: Vec<u16>,
    directory: Option<Vec<u16>>,
}

impl WindowsCommandLine {
    /// 将 Command 显式参数与环境编码。参数：command 为私有 env_clear 调用方配置。返回：有界宽字符输入或拒绝原因。
    pub(super) fn from_command(command: &Command) -> Result<Self, ChildError> {
        let executable = resolve_executable(command)?;
        let application = wide_nul(executable.as_os_str())?;
        let mut quoted = Vec::new();
        for (index, part) in std::iter::once(executable.as_os_str())
            .chain(command.get_args())
            .enumerate()
        {
            if index != 0 {
                quoted.push(b' ' as u16);
            }
            quote_crt(&wide(part)?, &mut quoted);
            if quoted.len() >= MAX_INPUT_UNITS {
                return Err(ChildError::InvalidLimits);
            }
        }
        quoted.push(0);
        let environment = explicit_environment(command)?;
        let directory = command
            .get_current_dir()
            .map(|path| {
                if !path.is_absolute() {
                    return Err(ChildError::Unsupported("relative probe working directory"));
                }
                wide_nul(path.as_os_str())
            })
            .transpose()?;
        Ok(Self {
            application,
            arguments: quoted,
            environment,
            directory,
        })
    }

    /// 借用绝对 .exe 的宽字符路径。参数：无。返回：NUL 结束的应用路径指针。
    pub(super) fn application(&self) -> *const u16 {
        self.application.as_ptr()
    }

    /// 借用可由 CreateProcessW 原位读取的命令行。参数：无。返回：可变 NUL 结束指针。
    pub(super) fn arguments(&mut self) -> *mut u16 {
        self.arguments.as_mut_ptr()
    }

    /// 借用只含 Command.get_envs 的双 NUL 环境块。参数：无。返回：环境块首地址。
    pub(super) fn environment(&self) -> *const std::ffi::c_void {
        self.environment.as_ptr().cast()
    }

    /// 借用显式绝对工作目录。参数：无。返回：工作目录指针，未指定时为空指针。
    pub(super) fn directory(&self) -> *const u16 {
        self.directory
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr())
    }
}

fn resolve_executable(command: &Command) -> Result<PathBuf, ChildError> {
    let program = command.get_program();
    let path = Path::new(program);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let name = program
            .to_str()
            .filter(|name| {
                !name.is_empty() && !name.chars().any(|unit| matches!(unit, '\\' | '/' | ':'))
            })
            .ok_or(ChildError::Unsupported(
                "probe program must be an absolute .exe or simple name",
            ))?;
        let name = if name.to_ascii_lowercase().ends_with(".exe") {
            name.to_owned()
        } else if name.contains('.') {
            return Err(ChildError::Unsupported(
                "probe scripts and non-.exe programs are forbidden",
            ));
        } else {
            format!("{name}.exe")
        };
        let explicit_path = command
            .get_envs()
            .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("PATH"))
            .and_then(|(_, value)| value)
            .ok_or(ChildError::Unsupported(
                "simple probe name requires explicit PATH",
            ))?;
        std::env::split_paths(explicit_path)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(&name))
            .find(|candidate| candidate.is_file())
            .ok_or(ChildError::Unsupported(
                "probe executable absent from explicit absolute PATH",
            ))?
    };
    let executable = candidate
        .canonicalize()
        .map_err(|error| ChildError::io("canonicalize probe executable", error))?;
    if !executable.is_file()
        || !executable
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(ChildError::Unsupported(
            "probe executable must be an .exe file",
        ));
    }
    let basename = executable
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if ["cmd.exe", "powershell.exe", "pwsh.exe"]
        .iter()
        .any(|shell| basename.eq_ignore_ascii_case(shell))
    {
        return Err(ChildError::Unsupported(
            "shell probe executables are forbidden",
        ));
    }
    Ok(executable)
}

fn explicit_environment(command: &Command) -> Result<Vec<u16>, ChildError> {
    let mut entries: Vec<(String, Vec<u16>, Vec<u16>)> = Vec::new();
    let mut encoded_units = 0usize;
    for (key, value) in command.get_envs() {
        let Some(value) = value else { continue };
        // 预留最终 NUL、当前条目的 '=' 和 NUL；每个字段最多只编码剩余额度加一单位。
        let available = MAX_INPUT_UNITS - encoded_units - 1;
        if available < 3 {
            return Err(ChildError::InvalidLimits);
        }
        let key_units = wide_bounded(key, available - 2)?;
        if key_units.is_empty() || key_units.contains(&(b'=' as u16)) {
            return Err(ChildError::Unsupported("invalid probe environment key"));
        }
        let value_units = wide_bounded(value, available - key_units.len() - 2)?;
        encoded_units += key_units.len() + value_units.len() + 2;
        entries.push((key.to_string_lossy().to_lowercase(), key_units, value_units));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(ChildError::Unsupported("duplicate probe environment keys"));
    }
    let mut block = Vec::new();
    for (_, key, value) in entries {
        block.extend(key);
        block.push(b'=' as u16);
        block.extend(value);
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    debug_assert_eq!(
        block.len(),
        if encoded_units == 0 {
            2
        } else {
            encoded_units + 1
        }
    );
    Ok(block)
}

fn wide(value: &OsStr) -> Result<Vec<u16>, ChildError> {
    wide_bounded(value, MAX_INPUT_UNITS - 1)
}

fn wide_bounded(value: &OsStr, max_units: usize) -> Result<Vec<u16>, ChildError> {
    let units: Vec<u16> = value.encode_wide().take(max_units + 1).collect();
    if units.contains(&0) || units.len() > max_units {
        return Err(ChildError::InvalidLimits);
    }
    Ok(units)
}

fn wide_nul(value: &OsStr) -> Result<Vec<u16>, ChildError> {
    let mut units = wide(value)?;
    units.push(0);
    Ok(units)
}

// CRT 规则：引号内的反斜杠在引号前成倍，末尾在闭合引号前成倍。
fn quote_crt(argument: &[u16], output: &mut Vec<u16>) {
    output.push(b'"' as u16);
    let mut slashes = 0usize;
    for &unit in argument {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else if unit == b'"' as u16 {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            output.push(unit);
            slashes = 0;
        } else {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            output.push(unit);
            slashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
}

#[cfg(test)]
mod tests {
    use super::{MAX_INPUT_UNITS, explicit_environment, quote_crt, wide};
    use crate::native_child::ChildError;
    use std::ffi::OsStr;
    use std::process::Command;

    #[test]
    fn crt_quotes_empty_unicode_backslashes_and_embedded_quotes() {
        let mut quoted = Vec::new();
        quote_crt(&[], &mut quoted);
        assert_eq!(quoted, vec![b'"' as u16, b'"' as u16]);

        quoted.clear();
        quote_crt(
            &[b'a' as u16, b'\\' as u16, b'"' as u16, b'b' as u16],
            &mut quoted,
        );
        assert_eq!(quoted, vec![34, 97, 92, 92, 92, 34, 98, 34]);

        quoted.clear();
        quote_crt(&[b'a' as u16, b'\\' as u16, b'\\' as u16], &mut quoted);
        assert_eq!(quoted, vec![34, 97, 92, 92, 92, 92, 34]);

        quoted.clear();
        let unicode: Vec<u16> = "😀".encode_utf16().collect();
        quote_crt(&unicode, &mut quoted);
        assert_eq!(&quoted[1..quoted.len() - 1], unicode.as_slice());
    }

    #[test]
    fn oversized_single_argument_is_rejected_at_utf16_limit() {
        let input = "长".repeat(1_000_000);
        assert!(matches!(
            wide(OsStr::new(&input)),
            Err(ChildError::InvalidLimits)
        ));
    }

    #[test]
    fn environment_accepts_exact_boundary_and_rejects_one_extra_unit() {
        let mut command = Command::new("ignored.exe");
        command
            .env_clear()
            .env("K", "x".repeat(MAX_INPUT_UNITS - 4));
        assert_eq!(
            explicit_environment(&command).unwrap().len(),
            MAX_INPUT_UNITS
        );
        command.env("K", "x".repeat(MAX_INPUT_UNITS - 3));
        assert!(matches!(
            explicit_environment(&command),
            Err(ChildError::InvalidLimits)
        ));
    }

    #[test]
    fn environment_limits_total_units_while_collecting_entries() {
        let mut command = Command::new("ignored.exe");
        command.env_clear();
        for index in 0..80 {
            command.env(format!("DG_{index:03}"), "x".repeat(512));
        }
        assert!(matches!(
            explicit_environment(&command),
            Err(ChildError::InvalidLimits)
        ));
    }
}
