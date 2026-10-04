//! 禁用执行操作的状态子命令目录的真实解析对象。
use clap::Subcommand;

#[cfg_attr(
    doc,
    doc = "禁用执行操作的状态子命令目录。来源：DiskGraph 原生 CLI main::OperationsAction；无 Java 对应实现。"
)]
#[derive(Subcommand)]
pub(crate) enum OperationsAction {
    List {
        #[arg(long)]
        scope: String,
    },
    Show {
        #[arg(long)]
        scope: String,
    },
    Cancel {
        #[arg(long)]
        scope: String,
    },
}
