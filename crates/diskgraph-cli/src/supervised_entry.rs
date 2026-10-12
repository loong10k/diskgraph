use crate::cli::Cli;
use crate::cli_exit::CliExit;

/// 参数：cli为实际解析的公开CLI；返回：既有JSON只读查询的监督状态或未支持拒绝。
/// 固定只读查询、默认本地身份和安装数据域；不构造本地Engine或绕过内部出生认证。
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
        CliExit::Supervised(
            diskgraph_engine::supervisor_service::run_query(
                socket,
                &crate::supervised_query::from_command(&cli.command)
                    .expect("validated read-only query"),
            )
            .unwrap_or_else(|error| {
                eprintln!("diskgraph: supervisor query failed: {error}");
                10
            }),
        )
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
        && crate::supervised_query::from_command(&cli.command).is_some()
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
    fn actual_cli_supervisor_route_admits_existing_read_only_query_commands() {
        for command in [
            vec!["scope", "list"],
            vec!["node", "--scope", "project"],
            vec!["search", "--scope", "project", "--pattern", "İSTANBUL"],
            vec!["snapshots", "--scope", "project"],
            vec!["status", "--job", "original-job"],
        ] {
            let mut arguments = vec![
                "diskgraph",
                "--json",
                "--supervisor-socket",
                "/run/diskgraph/front.sock",
            ];
            arguments.extend(command);
            assert!(allowed(&Cli::parse_from(arguments)));
        }
    }

    #[test]
    fn fixed_query_mapping_roundtrips_real_clap_without_global_argument_injection() {
        for command in [
            vec!["doctor"],
            vec!["scope", "list"],
            vec!["scope", "show", "--scope=a"],
            vec!["status", "--job=j"],
            vec!["snapshots", "--scope=a"],
            vec!["tree", "--scope=a", "--anonymize"],
            vec!["node", "--scope=a"],
            vec!["children", "--scope=a", "--unknown-only"],
            vec!["top", "--scope=a"],
            vec![
                "search",
                "--scope=--principal=attacker",
                "--pattern=İSTANBUL",
            ],
            vec!["explore", "--scope=a"],
            vec!["growth", "--scope=a", "--before=x", "--after=y"],
            vec!["changes", "--scope=a", "--before=x", "--after=y"],
            vec!["explain", "--scope=a", "--revision=r", "--entity=e"],
            vec![
                "related",
                "--scope=a",
                "--revision=r",
                "--entity=e",
                "--outgoing",
            ],
            vec!["impact", "--scope=a", "--revision=r", "--entity=e"],
            vec!["candidates", "--scope=a"],
        ] {
            let mut input = vec![
                "diskgraph",
                "--json",
                "--supervisor-socket",
                "/run/diskgraph/front.sock",
            ];
            input.extend(command);
            let first = Cli::parse_from(input);
            let query = crate::supervised_query::from_command(&first.command).unwrap();
            let mut role = vec![
                "diskgraph".to_owned(),
                "--json".to_owned(),
                "--data-dir=/fixed".to_owned(),
            ];
            role.extend(query.arguments().unwrap());
            let decoded = Cli::parse_from(role);
            assert_eq!(decoded.principal, "local-user");
            assert_eq!(decoded.data_dir, std::path::Path::new("/fixed"));
            assert_eq!(
                crate::supervised_query::from_command(&decoded.command).unwrap(),
                query
            );
        }
        for command in [
            vec!["scope", "add", "--root=/foreign"],
            vec!["index", "--scope=a"],
            vec!["grant", "--scope=a", "--content-read"],
            vec!["tree", "--scope=a", "--html=/foreign/report.html"],
            vec!["snapshots", "prune", "--scope=a", "--apply"],
        ] {
            let mut input = vec![
                "diskgraph",
                "--json",
                "--supervisor-socket",
                "/run/diskgraph/front.sock",
            ];
            input.extend(command);
            assert!(!allowed(&Cli::parse_from(input)));
        }
    }

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
