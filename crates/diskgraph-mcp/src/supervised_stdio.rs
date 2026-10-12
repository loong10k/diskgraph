use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

/// 参数：无；返回：监督stdio前端状态或普通MCP入口；监督路径不构造本地Engine。
pub(crate) fn run() -> Option<ExitCode> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let path = match parse(&arguments) {
        Ok(None) => return None,
        Ok(Some(path)) => path,
        Err(error) => {
            eprintln!("diskgraph-mcp: {error}");
            return Some(ExitCode::from(2));
        }
    };
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        if std::env::var_os("DISKGRAPH_LINUX_EXECUTOR_BIRTH").is_some()
            || std::env::var_os("DISKGRAPH_LINUX_EXECUTOR_FD").is_some()
            || std::env::var_os("DISKGRAPH_LINUX_SUPERVISOR_BIRTH").is_some()
            || std::env::var_os("DISKGRAPH_LINUX_SUPERVISOR_FD").is_some()
        {
            return Some(ExitCode::from(10));
        }
        Some(
            match diskgraph_engine::supervisor_service::run_mcp_session(&path) {
                Ok(status) => ExitCode::from(u8::try_from(status).unwrap_or(10)),
                Err(error) => {
                    eprintln!("diskgraph-mcp: guardian session failed: {error}");
                    ExitCode::from(10)
                }
            },
        )
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = path;
        eprintln!("diskgraph-mcp: guardian stdio unsupported on this platform");
        Some(ExitCode::from(10))
    }
}

fn parse(arguments: &[OsString]) -> io::Result<Option<PathBuf>> {
    if !arguments.iter().any(|argument| {
        argument
            .to_string_lossy()
            .starts_with("--supervisor-socket")
    }) {
        return Ok(None);
    }
    match arguments {
        [flag, path] if flag == "--supervisor-socket" && !path.is_empty() => Ok(Some(path.into())),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "guardian stdio requires exactly --supervisor-socket PATH; installation fixes data directory and profile",
        )),
    }
}

#[cfg(test)]
mod tests;
