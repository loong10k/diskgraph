//! DiskGraph CLI 聚合入口。业务类型与实现均在实际职责文件。
#[cfg(test)]
use cli::Cli;
#[cfg(test)]
use dispatch::dispatch;
use entry::cli_main;
use output::engine_business;
#[cfg(test)]
use scan_jobs::{wait_for_terminal, wait_for_terminal_until};
use std::process::ExitCode;

mod authorization;
mod candidate_commands;
mod cli;
mod cli_options;
mod command;
mod compare_support;
mod comparison_commands;
mod disabled_commands;
mod dispatch;
mod du_commands;
mod entry;
mod error_reply;
mod git_sync;
mod grant_commands;
mod history_commands;
mod html;
mod init_commands;
mod init_support;
mod install_command;
mod install_commands;
mod installer;
mod local;
mod node_commands;
mod operations_action;
mod output;
mod plan_action;
mod policy_command;
mod policy_commands;
#[cfg(test)]
mod query_terminal_tests;
mod relation_commands;
mod relation_reply;
mod scan_commands;
mod scan_jobs;
#[cfg(test)]
mod scan_terminal_tests;
mod scope_action;
mod scope_commands;
mod search_commands;
mod service_commands;
mod snapshot_action;
mod snapshot_commands;
mod snapshot_reply;
mod tree_commands;
mod tui;
mod tui_commands;
mod tui_frame_reader;
mod tui_request;
mod unsupported_command;

#[cfg(windows)]
fn main() -> ExitCode {
    // Windows 的默认主线程栈无法容纳 Debug 构建的大型命令分发栈帧。
    // 显式预留栈空间，让调试二进制与 Release 二进制使用同一业务路径。
    match std::thread::Builder::new()
        .name("diskgraph-cli".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(cli_main)
    {
        Ok(worker) => worker.join().unwrap_or(ExitCode::from(10)),
        Err(error) => {
            eprintln!("diskgraph: cannot start CLI worker: {error}");
            ExitCode::from(10)
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    cli_main()
}
