//! 公开 Store 导入的合成旧图；不提供原生扫描或无损根证明。
use diskgraph_core::{DiskGraph, DiskSnapshot, ResourceLocator, ScanCoverage, ScanSettings};

/// 参数：legacy_root 为旧字符串定位；返回：无原始身份列的单根元数据图。
pub(crate) fn legacy_graph(legacy_root: ResourceLocator) -> DiskGraph {
    DiskGraph {
        snapshot: DiskSnapshot {
            id: "legacy-snapshot".into(),
            root: legacy_root.clone(),
            volume_id: None,
            captured_at_unix_ms: 1,
            settings: ScanSettings {
                apparent_size: true,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: None,
                dedup_hardlinks: true,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes: vec![diskgraph_core::DiskNode {
            id: 1,
            parent_id: None,
            locator: legacy_root,
            name: "root".into(),
            kind: diskgraph_core::NodeKind::Directory,
            subtree_bytes: 0,
            direct_bytes: 0,
            size_known: true,
            files: 0,
            directories: 1,
            modified_unix_seconds: None,
            file_identity: None,
            category_hint: None,
            reclaim_hint: None,
            read_error: false,
        }],
        evidence: vec![],
    }
}
