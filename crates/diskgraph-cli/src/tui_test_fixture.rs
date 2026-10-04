//! TUI 真实授权与合法导入观测夹具；来源：OpenSpec Q-08 / 13.6。

use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, Grant, NodeKind, Permission, PolicyAuthorizer, PrincipalId,
    ResourceLocator, ScanCoverage, ScanSettings, ScopeId,
};
use diskgraph_engine::{Engine, EngineConfig};
use diskgraph_store::SqliteSnapshotStore;

use crate::tui_request::TuiRequest;

/// 通过公开暂存和发布接口建立已绑定范围的观测；不声称来自原生文件扫描。
/// 来源：DiskGraph 原生 Rust Q-08 测试；无 Java 对应实现。
pub(super) struct TuiFixture {
    _directory: tempfile::TempDir,
    pub(super) engine: Engine,
    pub(super) principal: PrincipalId,
    pub(super) scope: ScopeId,
    pub(super) policy: PolicyAuthorizer,
    pub(super) revision: String,
}

impl TuiFixture {
    /// 建立真实注册根、持久元数据授权和四节点合法观测。
    /// 参数：hidden_bytes 为各节点非展示回收提示长度，known_size 为大小覆盖状态。
    /// 返回：独立数据库及固定 revision；所有写入均调用公开 Store API。
    pub(super) fn new(hidden_bytes: usize, known_size: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            ..Default::default()
        })
        .unwrap();
        let principal = PrincipalId::new("tui-input-fixture").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        // 为末段单项撤权提供独立范围 grant，去掉管理员的 metadata 回退授权。
        {
            let mut control = engine.control_store().unwrap();
            let version = control.policy_version().unwrap();
            control
                .upsert_grant(&Grant {
                    principal: principal.clone(),
                    permission: Permission::MetadataRead,
                    scope: scope.clone(),
                    policy_version: version,
                })
                .unwrap();
            control
                .revoke_grant(
                    &principal,
                    &Permission::MetadataRead,
                    &diskgraph_engine::admin_scope(),
                )
                .unwrap();
        }
        let policy = engine.policy_authorizer().unwrap();
        let locator = ResourceLocator::NativePath(root.to_str().unwrap().to_owned());
        let root_node = DiskNode {
            id: 1,
            parent_id: None,
            locator: locator.clone(),
            name: "root".into(),
            kind: NodeKind::Directory,
            subtree_bytes: 30,
            direct_bytes: 0,
            size_known: known_size,
            files: 2,
            directories: 2,
            modified_unix_seconds: None,
            file_identity: None,
            category_hint: None,
            reclaim_hint: None,
            read_error: false,
        };
        let parent = DiskNode {
            id: 2,
            parent_id: Some(1),
            locator: ResourceLocator::NativePath(root.join("parent").to_str().unwrap().into()),
            name: "parent".into(),
            directories: 1,
            ..root_node.clone()
        };
        let leaf_a = DiskNode {
            id: 3,
            parent_id: Some(2),
            locator: ResourceLocator::NativePath(
                root.join("parent/leaf-a").to_str().unwrap().into(),
            ),
            name: "leaf-a".into(),
            kind: NodeKind::File,
            subtree_bytes: 10,
            direct_bytes: 10,
            files: 1,
            directories: 0,
            ..root_node.clone()
        };
        let leaf_b = DiskNode {
            id: 4,
            locator: ResourceLocator::NativePath(
                root.join("parent/leaf-b").to_str().unwrap().into(),
            ),
            name: "leaf-b".into(),
            subtree_bytes: 20,
            direct_bytes: 20,
            ..leaf_a.clone()
        };
        let revision = "tui-import".to_owned();
        let mut graph = DiskGraph {
            snapshot: DiskSnapshot {
                id: revision.clone(),
                root: locator,
                volume_id: Some("synthetic-tui-import-volume".into()),
                captured_at_unix_ms: 1,
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
            nodes: vec![root_node, parent, leaf_a, leaf_b],
            evidence: Vec::new(),
        };
        if hidden_bytes > 0 {
            for node in &mut graph.nodes {
                node.reclaim_hint = Some("x".repeat(hidden_bytes));
            }
        }
        let mut store =
            SqliteSnapshotStore::open(&directory.path().join("data/diskgraph.sqlite")).unwrap();
        store.append_staging_nodes(&revision, &graph.nodes).unwrap();
        store
            .publish_revision_owned(
                &revision,
                &graph,
                &revision,
                1,
                Some((engine.server_id().unwrap().as_str(), scope.as_str())),
            )
            .unwrap();
        drop(store);
        Self {
            _directory: directory,
            engine,
            principal,
            scope,
            policy,
            revision,
        }
    }

    /// 借用同一真实主体和持久授权建立 TUI 请求。
    /// 参数：无；返回：绑定已发布 revision 的请求，不建立替代授权器。
    pub(super) fn request(&self) -> TuiRequest<'_> {
        TuiRequest {
            engine: &self.engine,
            revision: &self.revision,
            principal: &self.principal,
            authorizer: &self.policy,
        }
    }
}
