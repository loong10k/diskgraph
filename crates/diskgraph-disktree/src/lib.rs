//! Read-only conversion from DiskTree's scanned tree to DiskGraph facts.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, NodeKind, ResourceLocator, ScanCoverage, ScanSettings,
};
use disktree_core::scan::ScanOptions;
use disktree_core::tree::{Node, NodeKind as DiskTreeNodeKind};
use uuid::Uuid;

/// Scan only the caller-selected native path. This function never deletes data.
pub fn scan_native(root: &Path, options: ScanOptions) -> io::Result<DiskGraph> {
    let root = root.canonicalize()?;
    let settings = ScanSettings {
        apparent_size: options.apparent_size,
        follow_links: options.follow_links,
        include_hidden: options.include_hidden,
        one_filesystem: options.one_filesystem,
        max_depth: options.max_depth,
        dedup_hardlinks: options.dedup_hardlinks,
    };
    let tree = disktree_core::scan::scan(&root, options)?;
    let captured_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis()
        .try_into()
        .map_err(io::Error::other)?;
    let mut nodes = Vec::new();
    let mut unreadable_nodes = 0;
    append_node(&tree, &root, None, &mut nodes, &mut unreadable_nodes);
    let depth_limited = settings.max_depth.is_some();
    Ok(DiskGraph {
        snapshot: DiskSnapshot {
            id: Uuid::new_v4().to_string(),
            root: ResourceLocator::NativePath(root.to_string_lossy().into_owned()),
            volume_id: volume_id(&root),
            captured_at_unix_ms,
            settings,
            coverage: ScanCoverage {
                complete: unreadable_nodes == 0 && !depth_limited,
                unreadable_nodes,
                depth_limited,
            },
        },
        nodes,
        evidence: Vec::new(),
    })
}

#[cfg(unix)]
fn volume_id(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.dev().to_string())
}

#[cfg(not(unix))]
fn volume_id(_path: &Path) -> Option<String> {
    // Windows volume identity needs a dedicated platform adapter.
    None
}

fn append_node(
    source: &Node,
    path: &Path,
    parent_id: Option<u64>,
    nodes: &mut Vec<DiskNode>,
    unreadable_nodes: &mut u64,
) {
    let id = nodes.len() as u64 + 1;
    if source.read_error {
        *unreadable_nodes += 1;
    }
    nodes.push(DiskNode {
        id,
        parent_id,
        locator: ResourceLocator::NativePath(path.to_string_lossy().into_owned()),
        name: source.name.to_string(),
        kind: match source.kind {
            DiskTreeNodeKind::Directory => NodeKind::Directory,
            DiskTreeNodeKind::File => NodeKind::File,
            DiskTreeNodeKind::Symlink => NodeKind::Symlink,
            DiskTreeNodeKind::Other => NodeKind::Other,
        },
        subtree_bytes: source.bytes,
        direct_bytes: source.own_bytes,
        files: source.files,
        directories: source.dirs,
        modified_unix_seconds: (source.modified > 0).then_some(source.modified),
        file_identity: None,
        category_hint: Some(source.category.label().to_owned()),
        reclaim_hint: source.reclaim.map(|hint| hint.label().to_owned()),
        read_error: source.read_error,
    });
    for child in &source.children {
        let child_path: PathBuf = path.join(child.name.as_ref());
        append_node(child, &child_path, Some(id), nodes, unreadable_nodes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_hidden_files_without_granting_delete_authority() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(".hidden"), b"hello").unwrap();
        let graph = scan_native(directory.path(), ScanOptions::default()).unwrap();
        assert!(graph.snapshot.coverage.complete);
        assert!(graph.nodes.iter().any(|node| node.name == ".hidden"));
        assert!(graph.evidence.is_empty());
        assert!(graph.candidates(1).is_empty());
    }

    #[test]
    fn depth_limited_scan_does_not_claim_completeness() {
        let directory = tempfile::tempdir().unwrap();
        let graph = scan_native(
            directory.path(),
            ScanOptions {
                max_depth: Some(1),
                ..ScanOptions::default()
            },
        )
        .unwrap();
        assert!(!graph.snapshot.coverage.complete);
    }
}
