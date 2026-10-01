/// 快照历史的显式维护动作，删除必须通过 --apply 启用。
#[derive(clap::Subcommand, Debug)]
pub(crate) enum SnapshotAction {
    /// Preview old-history reclamation, preserving latest, pins, and references.
    Prune {
        #[arg(long)]
        scope: String,
        #[arg(long, default_value_t = 1)]
        keep_last: u64,
        #[arg(long)]
        apply: bool,
    },
}
