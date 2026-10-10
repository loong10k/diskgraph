//! CLI entry 的真实职责实现。
use crate::cli::Cli;
use crate::cli_engine_host::CliEngineHost;
use crate::cli_exit::CliExit;
use crate::cli_options::scan_options_from;
use crate::command::Command;
use crate::dispatch::dispatch;
use crate::error_reply;
use crate::local::LocalIdentity;
use crate::output::engine_business;
use clap::Parser;
use diskgraph_core::{BusinessError, PrincipalId, QueryBudget};
use diskgraph_engine::{EngineConfig, EngineError};
use std::process::ExitCode;

/// 保留 cli_main 的原生业务职责与错误语义。来源：DiskGraph CLI main::cli_main。
/// 参数：与原入口的 cli_main 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn cli_main() -> CliExit {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Err(error) = diskgraph_engine::prepare_standalone_child_reaping() {
        eprintln!("diskgraph: child ownership startup failed: {error}");
        return CliExit::Code(ExitCode::from(10));
    }
    let cli = Cli::parse();
    // serve 必须在任何 CLI Engine/LocalIdentity 构造之前交接给 MCP。
    let outcome = if matches!(&cli.command, Command::Serve { .. }) {
        match crate::serve_command::run(&cli) {
            Ok(status) => return CliExit::Companion(status),
            Err(error) => Err(error),
        }
    } else {
        run(cli)
    };
    match outcome {
        Ok(()) => CliExit::Code(ExitCode::from(0)),
        Err(error) => {
            let business = engine_business(&error);
            if std::env::args().any(|arg| arg == "--json") {
                println!("{}", error_reply::line(&error));
            } else {
                eprintln!("{error}");
            }
            CliExit::Code(ExitCode::from(business.exit_code()))
        }
    }
}

/// 保留 run 的原生业务职责与错误语义。来源：DiskGraph CLI main::run。
/// 参数：与原入口的 run 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn run(cli: Cli) -> Result<(), EngineError> {
    if matches!(&cli.command, Command::Serve { .. }) {
        // 此命令仅由 cli_main 的进程转交入口处理，内部误路由也不得引导管理员。
        return Err(BusinessError::InvalidArgument.into());
    }
    // One knob drives both budget layers: the per-node charged ScanBudget
    // must not be tighter than the hard refusal ceiling, or a caller raising
    // the ceiling would still stop at the old charged limit (RT-02/RT-04).
    let host = CliEngineHost::open(EngineConfig {
        data_dir: cli.data_dir.clone(),
        max_nodes_per_scan: cli.max_nodes_per_scan,
        scan_budget: diskgraph_core::ScanBudget {
            max_nodes: cli.max_nodes_per_scan,
            max_staging_bytes: cli.max_staging_bytes,
            ..diskgraph_core::ScanBudget::default()
        },
        // Scan behavior mirrors disktree's own flags exactly: the snapshot
        // records these verbatim, and two snapshots are comparable only when
        // they were taken with the same options.
        scan_options: scan_options_from(&cli),
        ..EngineConfig::default()
    })
    .inspect_err(|_error| {
        // 标记故障发生在打开阶段；具体 SQLite 扩展错误码由原错误保留。
        eprintln!("diskgraph: engine startup failed");
    })?;
    host.execute(|engine| {
        #[cfg(test)]
        entry_deadline_tests::after_startup();
        // 启动准入已使用独立期限；查询从身份准备起计时，后续各阶段共享而不刷新。
        let deadline = diskgraph_core::query_deadline(QueryBudget::default())?;
        // 单次 CLI 查询不消费扫描队列；--wait 由当前请求执行，远程队列由长期服务执行。
        let principal = PrincipalId::new(cli.principal.clone())
            .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
        // The CLI authorizes against the local policy store; an empty policy is
        // default-deny, and first-run setup grants the local user administration.
        let authorizer = LocalIdentity::load(engine)?;
        let mut json_output = Vec::new();
        let outcome = dispatch(
            engine,
            &cli,
            &principal,
            &authorizer,
            &mut json_output,
            deadline,
        );
        if cli.json {
            for line in json_output {
                println!("{line}");
            }
        }
        outcome
    })
}

#[cfg(test)]
#[path = "entry_deadline_tests.rs"]
mod entry_deadline_tests;
