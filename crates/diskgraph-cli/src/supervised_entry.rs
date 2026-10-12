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
        let result = if matches!(cli.command, crate::command::Command::Serve { .. }) {
            diskgraph_engine::supervisor_service::run_mcp_session(socket)
        } else {
            diskgraph_engine::supervisor_service::run_query(
                socket,
                &crate::supervised_query::from_cli(cli).expect("validated read-only query"),
            )
        };
        CliExit::Supervised(result.unwrap_or_else(|error| {
            eprintln!("diskgraph: supervisor query failed: {error}");
            10
        }))
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
        && (cli.json || matches!(cli.command, crate::command::Command::Serve { .. }))
        && crate::supervised_query::from_cli(cli).is_some()
        && cli.principal == "local-user"
        && cli.data_dir == std::path::Path::new("diskgraph-data")
}

#[cfg(test)]
mod tests {
    use super::allowed;
    use crate::cli::Cli;
    use clap::Parser;
    #[test]
    fn guardian_cli_serve_is_stdio_protocol_without_json_envelope_and_rejects_overrides() {
        let cli = Cli::parse_from([
            "diskgraph",
            "--supervisor-socket=/fixed/front.sock",
            "serve",
        ]);
        assert!(allowed(&cli));
        assert_eq!(
            crate::supervised_query::from_cli(&cli)
                .unwrap()
                .arguments()
                .unwrap(),
            ["mcp"]
        );
        for options in [
            vec!["--transport=streamable-http"],
            vec!["--profile=all"],
            vec!["--host=0.0.0.0"],
            vec!["--port=9000"],
            vec!["--secure-transport"],
            vec!["--auth", "issuer", "audience", "secret"],
            vec!["--max-nodes-per-scan=100"],
        ] {
            let mut args = vec![
                "diskgraph",
                "--supervisor-socket=/fixed/front.sock",
                "serve",
            ];
            args.extend(options);
            assert!(!allowed(&Cli::parse_from(args)));
        }
    }
    #[test]
    fn supervised_scan_preserves_user_budgets_and_disktree_options() {
        for command in ["index", "sync"] {
            let first = Cli::parse_from([
                "diskgraph",
                "--json",
                "--supervisor-socket=/run/diskgraph/front.sock",
                "--max-nodes-per-scan=123",
                "--max-staging-bytes=456789",
                "-a",
                "-H",
                "-x",
                "-X",
                "--depth=7",
                "--no-dedup-hardlinks",
                command,
                "--scope=a",
                "--wait",
            ]);
            assert!(
                allowed(&first),
                "supervisor rejected requested scan options"
            );
            let query = crate::supervised_query::from_cli(&first).unwrap();
            let mut args = vec!["diskgraph".to_string()];
            args.extend(query.arguments().unwrap());
            let parsed = Cli::parse_from(args);
            assert_eq!(parsed.max_nodes_per_scan, first.max_nodes_per_scan);
            assert_eq!(parsed.max_staging_bytes, first.max_staging_bytes);
            assert_eq!(parsed.depth, first.depth);
            assert_eq!(parsed.apparent_size, first.apparent_size);
            assert_eq!(parsed.no_hidden, first.no_hidden);
            assert_eq!(parsed.one_filesystem, first.one_filesystem);
            assert_eq!(parsed.cross_filesystems, first.cross_filesystems);
            assert_eq!(parsed.no_dedup_hardlinks, first.no_dedup_hardlinks);
            assert_eq!(parsed.principal, "local-user");
            assert_eq!(crate::supervised_query::from_cli(&parsed).unwrap(), query);
        }
    }
    #[test]
    fn actual_cli_supervisor_route_admits_existing_read_only_query_commands() {
        for command in [
            vec!["scope", "list"],
            vec!["scope", "add", "--root=/fixture"],
            vec!["index", "--scope=a", "--wait"],
            vec!["sync", "--scope=a", "--wait"],
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
            vec!["scope", "add", "--root=/fixture"],
            vec!["index", "--scope=a", "--wait"],
            vec!["sync", "--scope=a", "--wait"],
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
            vec![
                "sync",
                "--scope=a",
                "--collector=git",
                "--revision=r",
                "--node-id=1",
            ],
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

#[cfg(test)]
#[cfg(unix)]
mod native_root_tests {
    use crate::{cli::Cli, command::Command, scope_action::ScopeAction};
    use clap::Parser;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn actual_clap_scope_registration_keeps_non_utf8_root_in_fixed_native_argument() {
        let original = std::ffi::OsString::from_vec(b"/tmp/registered-\xff".to_vec());
        let cli = Cli::parse_from([
            std::ffi::OsString::from("diskgraph"),
            "--json".into(),
            "--supervisor-socket=/run/diskgraph/front.sock".into(),
            "scope".into(),
            "add".into(),
            "--root".into(),
            original.clone(),
        ]);
        assert!(super::allowed(&cli));
        let query = crate::supervised_query::from_command(&cli.command).unwrap();
        let mut arguments = vec![std::ffi::OsString::from("diskgraph")];
        arguments.extend(query.native_arguments().unwrap());
        let parsed = Cli::parse_from(arguments);
        match parsed.command {
            Command::Scope {
                action: ScopeAction::Add { root },
            } => assert_eq!(root.as_os_str(), original),
            _ => panic!("native fixed arguments changed command"),
        }
        assert_eq!(parsed.principal, "local-user");
        assert_eq!(parsed.data_dir, std::path::Path::new("diskgraph-data"));
    }
}
