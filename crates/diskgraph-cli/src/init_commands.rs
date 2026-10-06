//! CLI init_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::cli_engine_host::CliEngineHost;
use crate::cli_options::scan_options_from;
use crate::command::Command;
use crate::init_support::absolute_data_dir;
use crate::init_support::init_index;
use crate::init_support::resolve_targets;
use crate::init_support::unsafe_root_reason;
use crate::installer;
use crate::output::envelope_line;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId};
use diskgraph_engine::{EngineConfig, EngineError};

/// 执行本组真实业务命令，保持原授权、预算、响应与副作用顺序。
/// 参数：本请求的解析参数及对应 Engine 依赖。返回：执行成功或原业务错误。
pub(crate) fn run(
    cli: &Cli,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Init {
            root,
            data_dir,
            targets,
            yes,
            force,
            location,
            no_instructions,
            index_only,
            uninstall,
            print_only,
        } => {
            // `init` is the only command that opens a store on a directory
            // the user did not name as a scope, so the root is resolved and
            // checked before the engine is even constructed.
            let root = root
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let root = std::fs::canonicalize(&root)
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            if !root.is_dir() {
                eprintln!(
                    "diskgraph: init needs a directory, and {} is not one",
                    root.display()
                );
                return Ok(());
            }
            if !*force && let Some(reason) = unsafe_root_reason(&root) {
                eprintln!("diskgraph: refusing to index {reason}");
                eprintln!("diskgraph: pass --force if that is what you meant");
                return Ok(());
            }
            // The data directory is resolved against the directory the user is
            // standing in, so `init` writes where they are looking.
            let data_dir = absolute_data_dir(data_dir);
            if data_dir != cli.data_dir {
                // A different directory than the global flag means this
                // command needs its own store; every other command takes one
                // --data-dir for all of them, so this is a one-off.
                eprintln!(
                    "diskgraph: init will use {} for the index; pass the same \
                     --data-dir to the other commands",
                    data_dir.display()
                );
            }
            let host = CliEngineHost::open(EngineConfig {
                data_dir: data_dir.clone(),
                max_nodes_per_scan: cli.max_nodes_per_scan,
                scan_budget: diskgraph_core::ScanBudget {
                    max_nodes: cli.max_nodes_per_scan,
                    max_staging_bytes: cli.max_staging_bytes,
                    ..diskgraph_core::ScanBudget::default()
                },
                scan_options: scan_options_from(cli),
                ..EngineConfig::default()
            })?;
            host.execute(|engine| {
            let summary = init_index(engine, &root, principal, authorizer)?;
            let chosen = resolve_targets(targets)?;
            let global = location == "global";
            let body = installer::instruction_body(&installer::locale_from_env());
            if *print_only {
                // The block a target would get, on stdout and nowhere else:
                // the way to read it before letting us write into a file you
                // have been keeping for a year.
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({
                        "index": summary,
                        "instructions": body,
                        "targets": chosen.iter().map(|target| target.name()).collect::<Vec<_>>(),
                    })),
                ));
                return Ok(());
            }
            if *uninstall {
                let mut removed = Vec::new();
                for file in installer::instruction_files(&chosen, &root, global) {
                    let outcome = installer::remove_block(&file.path)?;
                    removed.push(serde_json::json!({
                        "target": file.target.name(),
                        "path": file.path.display().to_string(),
                        "outcome": outcome.as_str(),
                    }));
                }
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({ "index": summary, "instructions": removed })),
                ));
                return Ok(());
            }
            if *no_instructions || *index_only || !*yes {
                if !*no_instructions && !*index_only && !*yes {
                    eprintln!(
                        "diskgraph: index ready; pass --yes to write the agent instruction files"
                    );
                }
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({ "index": summary })),
                ));
                return Ok(());
            }
            let mut written = Vec::new();
            let mut touched = 0_usize;
            for file in installer::instruction_files(&chosen, &root, global) {
                let outcome = installer::upsert_block(&file.path, body)?;
                touched += usize::from(outcome.wrote());
                written.push(serde_json::json!({
                    "target": file.target.name(),
                    "path": file.path.display().to_string(),
                    "outcome": outcome.as_str(),
                }));
            }
            if touched == 0 && !written.is_empty() {
                // A second init that changes nothing should say so, rather
                // than letting the file list read as if it had been rewritten.
                eprintln!("diskgraph: the agent files already say this; nothing written");
            }
            if let Some(kimi_config) = installer::kimi_mcp_config_path()
                && !installer::kimi_has_diskgraph(&kimi_config)
            {
                eprintln!(
                    "diskgraph: kimi reads MCP servers from {} - add diskgraph there with",
                    kimi_config.display()
                );
                eprintln!("diskgraph:   diskgraph serve --profile read-full");
            }
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({ "index": summary, "instructions": written })),
            ));
            Ok(())
            })
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
