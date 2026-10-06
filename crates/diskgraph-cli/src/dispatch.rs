//! CLI 类型路由。来源：原生 main::dispatch 的穷尽命令目录。
use crate::cli::Cli;
use crate::command::Command;
use diskgraph_core::{Authorizer, PrincipalId};
use diskgraph_engine::{Engine, EngineError};

/// 仅按命令类型交接职责，不执行查询或扫描。
/// 参数：本次 CLI 请求、共享 Engine、真实主体、授权器、响应缓冲和原始期限。返回：处理器执行结果。
pub(crate) fn dispatch(
    engine: &Engine,
    cli: &Cli,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
    deadline: std::time::Instant,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Scope { .. } => {
            crate::scope_commands::run(engine, cli, principal, authorizer, out)
        }
        Command::Index { .. } | Command::Sync { .. } | Command::Status { .. } => {
            crate::scan_commands::run(engine, cli, principal, authorizer, out)
        }
        Command::Snapshots { .. } => {
            crate::snapshot_commands::run(engine, cli, principal, authorizer, out)
        }
        Command::Tree { .. } => {
            crate::tree_commands::run(engine, cli, principal, authorizer, out, deadline)
        }
        Command::Node { .. }
        | Command::Children { .. }
        | Command::Top { .. }
        | Command::Explore { .. } => {
            crate::node_commands::run(engine, cli, principal, authorizer, out)
        }
        Command::Explain { .. } | Command::Related { .. } | Command::Impact { .. } => {
            crate::relation_commands::run(engine, cli, principal, authorizer, out, deadline)
        }
        Command::Growth { .. } | Command::Changes { .. } => {
            crate::history_commands::run(engine, cli, principal, authorizer, out, deadline)
        }
        Command::Compare { .. } => {
            crate::comparison_commands::run(engine, cli, principal, authorizer, out, deadline)
        }
        Command::Search { .. } => {
            crate::search_commands::run(engine, cli, principal, authorizer, out)
        }
        Command::Doctor => crate::service_commands::run(engine, cli, out),
        Command::Serve { .. } => Err(diskgraph_core::BusinessError::InvalidArgument.into()),
        Command::Policy(_) => crate::policy_commands::run(engine, cli, principal, authorizer, out),
        Command::Du { .. } => crate::du_commands::run(engine, cli, principal, authorizer, out),
        Command::Tui { .. } => crate::tui_commands::run(engine, cli, principal, authorizer),
        Command::Init { .. } => crate::init_commands::run(cli, principal, authorizer, out),
        Command::Candidates { .. } => {
            crate::candidate_commands::run(engine, cli, principal, authorizer, out, deadline)
        }
        Command::Grant { .. } => crate::grant_commands::run(engine, cli, principal, out),
        Command::Duplicates
        | Command::Read { .. }
        | Command::Move { .. }
        | Command::Copy { .. }
        | Command::Trash { .. }
        | Command::Restore { .. }
        | Command::Purge { .. }
        | Command::Plan { .. }
        | Command::Apply { .. }
        | Command::Operations { .. } => crate::disabled_commands::run(cli),
        Command::Install(action) => crate::install_commands::install_tool(action),
    }
}
