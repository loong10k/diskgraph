//! 合法短 revision / 可变 snapshot 标识夹具；来源：原生 Rust Q-08 准备预算契约。

use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, NodeKind, PolicyAuthorizer, PrincipalId, ResourceLocator,
    ScanCoverage, ScanSettings,
};
use diskgraph_engine::{Engine, EngineConfig};
use diskgraph_store::SqliteSnapshotStore;

/// 真实注册/授权配合公开 Store 导入的两份合成观测，不伪装原生扫描。
/// 来源：DiskGraph 原生 Rust 历史查询准备验收。
pub(crate) struct Fixture {
    _directory: tempfile::TempDir,
    pub(crate) engine: Engine,
    pub(crate) principal: PrincipalId,
    pub(crate) policy: PolicyAuthorizer,
}

impl Fixture {
    /// 参数：left_bytes/right_bytes 为合法 snapshot ID 长度；返回：短 revision 的已发布历史。
    pub(crate) fn new(left_bytes: usize, right_bytes: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("history-preparation").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let policy = engine.policy_authorizer().unwrap();
        let locator = ResourceLocator::NativePath(root.to_str().unwrap().into());
        let mut store =
            SqliteSnapshotStore::open(&directory.path().join("data/diskgraph.sqlite")).unwrap();
        for (revision, length, time, bytes) in
            [("left", left_bytes, 1, 10), ("right", right_bytes, 2, 15)]
        {
            let node = DiskNode {
                id: 1,
                parent_id: None,
                locator: locator.clone(),
                name: "root".into(),
                kind: NodeKind::Directory,
                subtree_bytes: bytes,
                direct_bytes: 0,
                size_known: true,
                files: 1,
                directories: 1,
                modified_unix_seconds: Some(100),
                file_identity: None,
                category_hint: None,
                reclaim_hint: None,
                read_error: false,
            };
            let file = DiskNode {
                id: 2,
                parent_id: Some(1),
                locator: ResourceLocator::NativePath(root.join("item").to_str().unwrap().into()),
                name: "item".into(),
                kind: NodeKind::File,
                direct_bytes: bytes,
                directories: 0,
                ..node.clone()
            };
            let graph = DiskGraph {
                snapshot: DiskSnapshot {
                    id: format!("{revision}-{}", "s".repeat(length)),
                    root: locator.clone(),
                    volume_id: Some("synthetic-import-volume".into()),
                    captured_at_unix_ms: time,
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
                nodes: vec![node, file],
                evidence: Vec::new(),
            };
            store.append_staging_nodes(revision, &graph.nodes).unwrap();
            store
                .publish_revision_owned(
                    revision,
                    &graph,
                    revision,
                    time,
                    Some((engine.server_id().unwrap().as_str(), scope.as_str())),
                )
                .unwrap();
        }
        drop(store);
        Self {
            _directory: directory,
            engine,
            principal,
            policy,
        }
    }
}
