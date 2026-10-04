//! 禁用写操作的计划子命令目录的真实解析对象。
use clap::Subcommand;

#[cfg_attr(
    doc,
    doc = "禁用写操作的计划子命令目录。来源：DiskGraph 原生 CLI main::PlanAction；无 Java 对应实现。"
)]
#[derive(Subcommand)]
pub(crate) enum PlanAction {
    Show {
        #[arg(long)]
        scope: String,
    },
    Validate {
        #[arg(long)]
        scope: String,
    },
    Create {
        #[arg(long)]
        scope: String,
    },
}
