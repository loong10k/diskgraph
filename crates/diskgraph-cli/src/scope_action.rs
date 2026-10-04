//! 注册范围管理子命令的真实解析对象。
use clap::Subcommand;
use std::path::PathBuf;

#[cfg_attr(
    doc,
    doc = "注册范围管理子命令。来源：DiskGraph 原生 CLI main::ScopeAction；无 Java 对应实现。"
)]
#[derive(Subcommand)]
pub(crate) enum ScopeAction {
    /// Register (or idempotently return) a scope for a native root.
    Add {
        #[arg(long)]
        root: PathBuf,
    },
    /// List registered scopes.
    List,
    /// Show one scope.
    Show {
        #[arg(long)]
        scope: String,
    },
    /// Revoke a scope; never deletes files or history.
    Remove {
        #[arg(long)]
        scope: String,
    },
}
