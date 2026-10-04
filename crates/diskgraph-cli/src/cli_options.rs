//! CLI cli_options 的真实职责实现。
use crate::cli::Cli;

/// 保留 scan_options_from 的原生业务职责与错误语义。来源：DiskGraph CLI main::scan_options_from。
/// 参数：与原入口的 scan_options_from 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn scan_options_from(cli: &Cli) -> diskgraph_disktree_core::scan::ScanOptions {
    diskgraph_disktree_core::scan::ScanOptions {
        apparent_size: cli.apparent_size,
        follow_links: false,
        include_hidden: !cli.no_hidden,
        one_filesystem: !cli.cross_filesystems || cli.one_filesystem,
        max_depth: cli.depth,
        dedup_hardlinks: !cli.no_dedup_hardlinks,
        ..diskgraph_disktree_core::scan::ScanOptions::default()
    }
}
