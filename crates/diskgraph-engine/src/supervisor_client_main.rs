//! 原公开连接的有限前端入口；不把协议EOF当作业务成功。
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 || args[0] != "--socket" {
        return std::process::ExitCode::from(2);
    }
    match diskgraph_engine::supervisor_service::run_client(std::path::Path::new(&args[1])) {
        Ok(code) => {
            std::process::exit(code);
        }
        // 原预算到期后不再对可能阻塞的用户stderr补写诊断。
        Err(_) => std::process::ExitCode::from(10),
    }
}

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(10)
}
