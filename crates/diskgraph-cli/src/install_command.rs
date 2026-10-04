//! 客户端配置管理子命令的真实解析对象。
use clap::Subcommand;
use std::path::PathBuf;

#[cfg_attr(
    doc,
    doc = "客户端配置管理子命令。来源：DiskGraph 原生 CLI main::InstallCommand；无 Java 对应实现。"
)]
/// Client configuration management (P3). Writes are previewed by default and
/// applied only with `--apply-config`.
#[derive(Subcommand)]
pub(crate) enum InstallCommand {
    /// Register the MCP server into a client configuration.
    Add {
        #[arg(long)]
        client: String,
        /// Path to the client configuration file.
        #[arg(long)]
        config: PathBuf,
        /// Write the change; without this the diff is only reported.
        #[arg(long)]
        apply_config: bool,
    },
    /// Report whether DiskGraph is registered, without changing anything.
    Show {
        #[arg(long)]
        client: String,
        #[arg(long)]
        config: PathBuf,
    },
    /// Remove only the entry DiskGraph owns.
    Remove {
        #[arg(long)]
        client: String,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        apply_config: bool,
    },
}
