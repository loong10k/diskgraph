//! 全局参数与业务命令解析入口的真实解析对象。
use crate::command::Command;
use clap::Parser;
use std::path::PathBuf;

#[cfg_attr(
    doc,
    doc = "全局参数与业务命令解析入口。来源：DiskGraph 原生 CLI main::Cli；无 Java 对应实现。"
)]
/// DiskGraph — shared Rust file-relationship engine (read-only CLI surface).
#[derive(Parser)]
#[command(
    name = "diskgraph",
    version,
    about,
    after_long_help = include_str!("../QUICKSTART.md"),
    disable_help_subcommand = false
)]
pub(crate) struct Cli {
    /// Data directory holding diskgraph.sqlite and diskgraph-control.sqlite.
    #[arg(long, default_value = "diskgraph-data", global = true)]
    pub(crate) data_dir: PathBuf,
    /// Machine-readable output; stdout carries JSON only, logs go to stderr.
    #[arg(long, global = true)]
    pub(crate) json: bool,
    /// Principal the CLI acts as (default single-user identity).
    #[arg(long, global = true, default_value = "local-user")]
    pub(crate) principal: String,
    /// Hard node ceiling for one scan (RT-04). A walk past it refuses to
    /// publish instead of returning partial data silently.
    #[arg(long, global = true, default_value_t = 2_000_000)]
    pub(crate) max_nodes_per_scan: u64,
    /// Encoded metadata staging budget for one scan (default 2 GiB).
    /// Observed file sizes do not consume this storage budget.
    #[arg(long, global = true, default_value_t = 2 << 30)]
    pub(crate) max_staging_bytes: u64,
    /// Measure apparent length instead of allocated blocks (disktree -a).
    #[arg(short = 'a', long, global = true)]
    pub(crate) apparent_size: bool,
    /// Skip dotfiles and dot-directories (disktree -H).
    #[arg(short = 'H', long, global = true)]
    pub(crate) no_hidden: bool,
    /// Stay on the root's volume (disktree -x; this is the default).
    #[arg(short = 'x', long, global = true)]
    pub(crate) one_filesystem: bool,
    /// Cross filesystem boundaries (disktree -X).
    #[arg(short = 'X', long, global = true)]
    pub(crate) cross_filesystems: bool,
    /// Stop descending past this depth (disktree -d); totals below it are
    /// recorded as unknown rather than estimated.
    #[arg(short = 'd', long, global = true)]
    pub(crate) depth: Option<usize>,
    /// Count a hardlinked file once per link instead of once (disktree's
    /// dedup_hardlinks=false; the default matches disktree's true).
    #[arg(long, global = true)]
    pub(crate) no_dedup_hardlinks: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}
