//! Read-only conversion from DiskTree's scanned tree to DiskGraph facts.
//!
//! The v2 entry point ([`scan_native_v2`]) captures lossless locators, file
//! identity, and per-node observation times (P1 tasks 2.5–2.7, specs FS-01 to
//! FS-03). The v1 [`scan_native`] stays as the compatibility projection.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, FileIdentity, Locator, NodeKind,
    ResourceLocator, ScanCoverage, ScanSettings,
};
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_disktree_core::tree::{Node, NodeKind as DiskTreeNodeKind};
use uuid::Uuid;

/// One scanned node: the v1 projection plus lossless v2 identity data.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeV2 {
    /// The v1 [`DiskNode`] exactly as older consumers expect it.
    pub v1: DiskNode,
    /// Lossless locator; raw bytes survive non-UTF-8 names on byte-transparent
    /// filesystems, while `display` is the v1-style lossy rendering.
    pub locator: Locator,
    /// Volume + file identity when the platform reports it; never a permanent ID.
    pub identity: Option<FileIdentity>,
    /// The node's own mtime, separate from any subtree aggregation (FS-03).
    pub self_modified: Option<i64>,
}

/// A v2 scan: the same observation window as v1 with richer per-node facts.
#[derive(Clone, Debug, PartialEq)]
pub struct ScanResultV2 {
    pub snapshot: DiskSnapshot,
    pub nodes: Vec<NodeV2>,
    /// Always empty in P1: no collector ships yet, so candidates stay empty.
    pub evidence: Vec<EvidenceEdge>,
}

/// Scan only the caller-selected native path (v1 projection). This function
/// never deletes data.
pub fn scan_native(root: &Path, options: ScanOptions) -> io::Result<DiskGraph> {
    let result = scan_native_v2(root, options)?;
    Ok(DiskGraph {
        snapshot: result.snapshot,
        nodes: result.nodes.into_iter().map(|node| node.v1).collect(),
        evidence: result.evidence,
    })
}

/// Scan with v2 identity capture. Symlinks are never followed by default and
/// one-filesystem stays on, so scans neither leave the observed volume nor
/// cross link boundaries (FS-05). Cloud placeholders are only stat'ed here;
/// download-triggering observation is deferred to platform adapters and listed
/// in `diskgraph-testkit`'s real-OS requirements.
pub fn scan_native_v2(root: &Path, options: ScanOptions) -> io::Result<ScanResultV2> {
    let root = root.canonicalize()?;
    let settings = ScanSettings {
        apparent_size: options.apparent_size,
        follow_links: options.follow_links,
        include_hidden: options.include_hidden,
        one_filesystem: options.one_filesystem,
        max_depth: options.max_depth,
        dedup_hardlinks: options.dedup_hardlinks,
    };
    let tree = diskgraph_disktree_core::scan::scan(&root, options)?;
    convert_tree(&root, &tree, settings)
}

/// Converts an already-scanned DiskTree node tree into v2 facts. The engine's
/// cancellable scan path spawns the walk itself and calls this when the tree
/// is ready, so observation window and settings stay identical to the
/// blocking entry point (FS-01).
pub fn convert_tree(root: &Path, tree: &Node, settings: ScanSettings) -> io::Result<ScanResultV2> {
    let root = root.canonicalize()?;
    let captured_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis()
        .try_into()
        .map_err(io::Error::other)?;
    let mut nodes = Vec::new();
    let mut unreadable_nodes = 0;
    append_node(tree, &root, None, &mut nodes, &mut unreadable_nodes);
    let depth_limited = settings.max_depth.is_some();
    Ok(ScanResultV2 {
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
    // Windows volume identity needs a dedicated platform adapter (FS-02).
    None
}

/// Captures identity and the node's own mtime with one non-following stat.
/// This costs one extra metadata call per node; correctness beats the syscall.
fn observe(path: &Path) -> (Option<FileIdentity>, Option<i64>) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let identity = FileIdentity {
                    volume_id: metadata.dev().to_string(),
                    file_id: metadata.ino(),
                };
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .and_then(|duration| i64::try_from(duration.as_secs()).ok());
                (Some(identity), modified)
            }
            #[cfg(not(unix))]
            {
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .and_then(|duration| i64::try_from(duration.as_secs()).ok());
                (None, modified)
            }
        }
        Err(_) => (None, None),
    }
}

fn append_node(
    source: &Node,
    path: &Path,
    parent_id: Option<u64>,
    nodes: &mut Vec<NodeV2>,
    unreadable_nodes: &mut u64,
) {
    // 显式栈保持上游 preorder 顺序，转换不依赖调用栈深度。
    let mut pending = vec![(source, path.to_path_buf(), parent_id)];
    while let Some((source, path, parent_id)) = pending.pop() {
        let id = nodes.len() as u64 + 1;
        if source.read_error {
            *unreadable_nodes += 1;
        }
        let (identity, self_modified) = observe(&path);
        let v1_identity = identity.clone();
        nodes.push(NodeV2 {
            locator: Locator::from_native_path(&path),
            identity,
            self_modified,
            v1: DiskNode {
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
                // A scanned node with a recorded byte count is a known size; the
                // upstream scanner never reports an unknown size today.
                size_known: true,
                direct_bytes: source.own_bytes,
                files: source.files,
                directories: source.dirs,
                modified_unix_seconds: (source.modified > 0).then_some(source.modified),
                file_identity: v1_identity,
                category_hint: Some(source.category.label().to_owned()),
                reclaim_hint: source.reclaim.map(|hint| hint.label().to_owned()),
                read_error: source.read_error,
            },
        });
        for child in source.children.iter().rev() {
            let child_path: PathBuf = path.join(child.name.as_ref());
            pending.push((child, child_path, Some(id)));
        }
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

    #[test]
    fn v2_nodes_carry_identity_locator_and_self_mtime() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("data.bin"), b"payload").unwrap();
        let result = scan_native_v2(directory.path(), ScanOptions::default()).unwrap();
        assert!(result.evidence.is_empty());
        let file = result
            .nodes
            .iter()
            .find(|node| node.v1.name == "data.bin")
            .unwrap();
        // Lossless locator: raw bytes end with the file name.
        let raw = file.locator.raw_bytes().unwrap();
        assert!(raw.ends_with(b"data.bin"));
        // Self mtime is observed independently of the subtree aggregate.
        assert!(file.self_modified.is_some());
        // v1 projection stays aligned with the v2 locator display.
        assert_eq!(
            file.v1.locator,
            ResourceLocator::NativePath(file.locator.display.clone())
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_identity_matches_stat_on_unix() {
        use std::os::unix::fs::MetadataExt;

        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("f"), b"x").unwrap();
        let result = scan_native_v2(directory.path(), ScanOptions::default()).unwrap();
        let file = result
            .nodes
            .iter()
            .find(|node| node.v1.name == "f")
            .unwrap();
        let metadata = std::fs::symlink_metadata(directory.path().join("f")).unwrap();
        let identity = file.identity.as_ref().expect("identity must be captured");
        assert_eq!(identity.volume_id, metadata.dev().to_string());
        assert_eq!(identity.file_id, metadata.ino());
        // The v1 projection shares the same identity value.
        assert_eq!(file.v1.file_identity.as_ref(), Some(identity));
    }
}
