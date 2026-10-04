//! 策略版本与撤销子命令的真实解析对象。
use clap::Subcommand;

#[cfg_attr(
    doc,
    doc = "策略版本与撤销子命令。来源：DiskGraph 原生 CLI main::PolicyCommand；无 Java 对应实现。"
)]
#[derive(Subcommand)]
pub(crate) enum PolicyCommand {
    /// Publish a new policy version; old-version grants and cursors expire.
    Publish {
        #[arg(long)]
        version: u64,
    },
    /// Revoke the whole policy; nothing is authorized until republished.
    Revoke,
}
