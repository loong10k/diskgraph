use crate::cli::Cli;
use crate::cli_exit::CliExit;
use crate::command::Command;

/// 参数：cli为实际解析的公开CLI；返回：既有JSON doctor的监督状态或未支持拒绝。
/// 首期固定doctor、默认本地身份和安装数据域；不构造本地Engine或绕过内部出生认证。
pub(crate) fn run(cli: &Cli) -> CliExit {
    if !allowed(cli) {
        return CliExit::Supervised(10);
    }
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        if std::env::var_os("DISKGRAPH_LINUX_SUPERVISOR_BIRTH").is_some()
            || std::env::var_os("DISKGRAPH_LINUX_SUPERVISOR_FD").is_some()
        {
            return CliExit::Supervised(10);
        }
        let socket = cli
            .supervisor_socket
            .as_deref()
            .expect("checked public supervisor entry");
        CliExit::Supervised(diskgraph_engine::supervisor_service::run_client(socket).unwrap_or(10))
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        CliExit::Supervised(10)
    }
}

fn allowed(cli: &Cli) -> bool {
    cli.supervisor_socket.is_some()
        && cli.json
        && matches!(&cli.command, Command::Doctor)
        && cli.principal == "local-user"
        && cli.data_dir == std::path::Path::new("diskgraph-data")
        && cli.max_nodes_per_scan == 2_000_000
        && cli.max_staging_bytes == 2 << 30
        && !cli.apparent_size
        && !cli.no_hidden
        && !cli.one_filesystem
        && !cli.cross_filesystems
        && cli.depth.is_none()
        && !cli.no_dedup_hardlinks
}

#[cfg(test)]
mod tests {
    use super::allowed;
    use crate::cli::Cli;
    use clap::Parser;
    #[test]
    fn actual_cli_supervisor_route_requires_fixed_doctor_without_ignored_overrides() {
        let prefix = [
            "diskgraph",
            "--supervisor-socket",
            "/run/diskgraph/frontend.sock",
            "--json",
        ];
        let mut valid = prefix.to_vec();
        valid.push("doctor");
        assert!(allowed(&Cli::parse_from(valid)));
        for args in [
            vec!["doctor", "--principal", "other"],
            vec!["doctor", "--data-dir", "/foreign"],
            vec!["doctor", "--max-nodes-per-scan", "10"],
            vec!["doctor", "-H"],
        ] {
            let mut input = prefix.to_vec();
            input.extend(args);
            assert!(!allowed(&Cli::parse_from(input)));
        }
        assert!(!allowed(&Cli::parse_from([
            "diskgraph",
            "--supervisor-socket",
            "/run/diskgraph/frontend.sock",
            "doctor"
        ])));
    }
}
