//! 独立查询契约的合法导入观测，不替代真实扫描测试。

use crate::McpService;
use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, NodeKind, ResourceLocator, ScanCoverage, ScanSettings,
};
use diskgraph_store::SqliteSnapshotStore;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static REVISION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 发布查询测试所需的有限、显式节点观测。
/// 来源：原生 Rust MCP Q-01/SC-04 测试；无 Java 对应对象。
pub(super) struct QueryRevisionFixture;

impl QueryRevisionFixture {
    /// 注册真实范围并通过公开 staging/归属发布事务导入固定项目观测。
    /// 参数：service 为真实服务，root 为保活夹具根目录；返回：已注册 scope。
    /// 不创建或完成扫描 job，不采集关系，不证明原生扫描能力。
    pub(super) fn publish(service: &mut McpService, root: &Path) -> String {
        let root = root.canonicalize().unwrap();
        let scope = service
            .engine()
            .register_scope(
                &root,
                service.context.principal(),
                &service.authorizer().unwrap(),
            )
            .unwrap();
        let sequence = REVISION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let revision = format!("mcp-query-import-{sequence}");
        let locator = |path: &Path| ResourceLocator::NativePath(path.to_str().unwrap().to_owned());
        let mut nodes = vec![DiskNode {
            id: 1,
            parent_id: None,
            locator: locator(&root),
            name: root.file_name().unwrap().to_str().unwrap().to_owned(),
            kind: NodeKind::Directory,
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
        }];
        // 只描述既有查询夹具的四个固定条目；不遍历目录或替代 pinned scanner。
        for (id, parent_id, relative, kind) in [
            (2, 1, "Cargo.toml", NodeKind::File),
            (3, 1, "target", NodeKind::Directory),
            (4, 3, "target/bin", NodeKind::File),
            (5, 1, "new.txt", NodeKind::File),
        ] {
            let path = root.join(relative);
            if !path.exists() {
                continue;
            }
            let metadata = std::fs::metadata(&path).unwrap();
            assert_eq!(metadata.is_dir(), kind == NodeKind::Directory);
            let bytes = if kind == NodeKind::File {
                metadata.len()
            } else {
                0
            };
            nodes.push(DiskNode {
                id,
                parent_id: Some(parent_id),
                locator: locator(&path),
                name: path.file_name().unwrap().to_str().unwrap().to_owned(),
                kind,
                subtree_bytes: bytes,
                direct_bytes: bytes,
                files: u64::from(kind == NodeKind::File),
                directories: u64::from(kind == NodeKind::Directory),
                ..nodes[0].clone()
            });
        }
        // 固定树中子节点排在父节点之后，逆序聚合保留完整目录计数与容量。
        for index in (1..nodes.len()).rev() {
            let child = nodes[index].clone();
            let parent = nodes
                .iter_mut()
                .find(|node| Some(node.id) == child.parent_id)
                .unwrap();
            parent.subtree_bytes += child.subtree_bytes;
            parent.files += child.files;
            parent.directories += child.directories;
        }
        let graph = DiskGraph {
            snapshot: DiskSnapshot {
                id: revision.clone(),
                root: locator(&root),
                volume_id: None,
                captured_at_unix_ms: sequence,
                settings: ScanSettings {
                    apparent_size: true,
                    follow_links: false,
                    include_hidden: true,
                    one_filesystem: true,
                    max_depth: None,
                    dedup_hardlinks: false,
                },
                coverage: ScanCoverage {
                    complete: true,
                    unreadable_nodes: 0,
                    depth_limited: false,
                },
            },
            nodes,
            evidence: Vec::new(),
        };
        let mut store =
            SqliteSnapshotStore::open(&service.engine().data_dir().join("diskgraph.sqlite"))
                .unwrap();
        store.append_staging_nodes(&revision, &graph.nodes).unwrap();
        store
            .publish_revision_owned(
                &revision,
                &graph,
                &revision,
                sequence,
                Some((
                    service.engine().server_id().unwrap().as_str(),
                    scope.as_str(),
                )),
            )
            .unwrap();
        scope.as_str().to_owned()
    }
}
