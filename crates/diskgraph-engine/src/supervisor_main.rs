//! 固定安装Linux监督broker入口；不安装桌面服务或授予其他平台执行能力。
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !(args.len() == 2 || args.len() == 3)
        || args[0] != "--config"
        || (args.len() == 3 && args[2] != "--once" && args[2] != "--connection")
    {
        eprintln!(
            "usage: diskgraph-supervisor --config ROOT_INSTALLED_CONFIG [--once | --connection]"
        );
        return std::process::ExitCode::from(2);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let result = if args.len() == 3 && args[2] == "--connection" {
        use std::os::fd::{FromRawFd, OwnedFd};
        // 固定socket激活入口独占消费stdin；实际Unix类型和客户凭据由guardian先行核验。
        if unsafe { libc::fcntl(0, libc::F_GETFD) } < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            let original = unsafe { OwnedFd::from_raw_fd(0) };
            diskgraph_engine::supervisor_service::LinuxGuardian::serve_accepted(
                std::path::Path::new(&args[1]),
                std::os::unix::net::UnixStream::from(original),
                deadline,
            )
        }
    } else {
        diskgraph_engine::supervisor_service::LinuxBroker::open(
            std::path::Path::new(&args[1]),
            deadline,
        )
        .and_then(|broker| broker.serve(args.len() == 3, deadline))
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("diskgraph-supervisor: {error}");
            std::process::ExitCode::from(10)
        }
    }
}

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
fn main() -> std::process::ExitCode {
    eprintln!("diskgraph-supervisor: platform service admission is not qualified");
    std::process::ExitCode::from(10)
}
