//! 注册本平台编码的离线旧原生元数据，不在当前宿主创建原始定位目录。
use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, Grant, Locator, NodeKind, Permission, PrincipalId,
    ResourceLocator, ScanCoverage, ScanSettings, ScopeId,
};
use diskgraph_engine::Engine;
use diskgraph_store::SqliteSnapshotStore;
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

/// 旧原生有归属历史的 FFI 隔离夹具；来源：Q-04 / D34 合法离线导入契约。
pub(crate) struct NativeGrowthScopeFixture {
    _directory: tempfile::TempDir,
    pub(crate) database: String,
    pub(crate) engine: Engine,
    pub(crate) principal: PrincipalId,
    pub(crate) scopes: [ScopeId; 2],
    roots: [PathBuf; 2],
}

impl NativeGrowthScopeFixture {
    /// 注册两份真实原始定位元数据；参数：无；返回：双侧已授权且归属不同的旧记录。
    pub(crate) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database = directory
            .path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned();
        let engine = crate::open_engine(&database).unwrap();
        let principal = crate::local_principal().unwrap();
        // Unix 使用不同非法 UTF-8 字节；Windows 使用不同未配对 UTF-16 码元。
        // 两者都由系统原生类型自然产生相同显示值，仅导入离线元数据，不访问这些根。
        let roots = raw_names().map(|name| offline_root().join(name));
        let scopes = {
            let mut control = engine.control_store().unwrap();
            let version = control.policy_version().unwrap();
            roots.each_ref().map(|root| {
                let locator = Locator::from_native_path(root);
                let scope = control.register_scope(&locator, Some("2049")).unwrap();
                control
                    .upsert_grant(&Grant {
                        principal: principal.clone(),
                        permission: Permission::MetadataRead,
                        scope: scope.clone(),
                        policy_version: version,
                    })
                    .unwrap();
                assert_eq!(
                    control.register_scope(&locator, Some("2049")).unwrap(),
                    scope
                );
                scope
            })
        };
        assert_ne!(scopes[0], scopes[1]);
        let first = engine.scope(&scopes[0]).unwrap();
        let second = engine.scope(&scopes[1]).unwrap();
        assert_ne!(first.root.raw_b64, second.root.raw_b64);
        assert_eq!(first.root.display, second.root.display);
        assert_eq!(first.volume_id, second.volume_id);
        let mut fixture = Self {
            _directory: directory,
            database,
            engine,
            principal,
            scopes,
            roots,
        };
        fixture.publish("before", 0, 100, 1, None);
        fixture.publish("after", 1, 200, 2, None);
        // 重新打开真实 FFI realm 后，旧显式归属不可被相同显示值覆盖。
        fixture.engine = crate::open_engine(&fixture.database).unwrap();
        for (revision, scope) in [
            ("before", &fixture.scopes[0]),
            ("after", &fixture.scopes[1]),
        ] {
            assert_eq!(
                fixture
                    .engine
                    .authorize_revision(
                        Some(scope),
                        revision,
                        &fixture.principal,
                        &fixture.engine.policy_authorizer().unwrap(),
                    )
                    .unwrap(),
                *scope
            );
            assert_eq!(
                fixture
                    .engine
                    .revision_reader()
                    .unwrap()
                    .revision_ownership(revision)
                    .unwrap()
                    .unwrap(),
                (
                    fixture.engine.server_id().unwrap().to_string(),
                    scope.to_string()
                )
            );
        }
        fixture
    }

    /// 通过可信旧节点发布入口导入记录；参数：标识、根序号、大小、时间、可选归属服务；返回：无。
    pub(crate) fn publish(
        &self,
        id: &str,
        side: usize,
        bytes: u64,
        time: u64,
        server: Option<&str>,
    ) {
        let graph = stored_graph(id, &self.roots[side], bytes, time, "2049".into());
        let mut store = SqliteSnapshotStore::open(Path::new(&self.database)).unwrap();
        store.append_staging_nodes(id, &graph.nodes).unwrap();
        let local = self.engine.server_id().unwrap();
        store
            .publish_revision_owned(
                id,
                &graph,
                id,
                time,
                Some((server.unwrap_or(local.as_str()), self.scopes[side].as_str())),
            )
            .unwrap();
    }

    /// 编码旧协议原始显示定位；参数：是否根节点；返回：合法 locator JSON。
    pub(crate) fn locator_json(&self, root: bool) -> String {
        let path = if root {
            self.roots[0].clone()
        } else {
            self.roots[0].join("item")
        };
        serde_json::to_string(&ResourceLocator::NativePath(
            path.to_string_lossy().into_owned(),
        ))
        .unwrap()
    }

    /// 撤销一侧实际授权；参数：scope 序号；返回：无。
    pub(crate) fn revoke(&self, side: usize) {
        self.engine
            .control_store()
            .unwrap()
            .revoke_grant(
                &self.principal,
                &Permission::MetadataRead,
                &self.scopes[side],
            )
            .unwrap();
    }
}

fn stored_graph(id: &str, root: &Path, bytes: u64, time: u64, volume_id: String) -> DiskGraph {
    let locator = ResourceLocator::NativePath(root.to_string_lossy().into_owned());
    let directory = DiskNode {
        id: 1,
        parent_id: None,
        locator: locator.clone(),
        name: root.file_name().unwrap().to_string_lossy().into_owned(),
        kind: NodeKind::Directory,
        subtree_bytes: bytes,
        direct_bytes: 0,
        size_known: true,
        files: 1,
        directories: 1,
        modified_unix_seconds: None,
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    };
    let file = DiskNode {
        id: 2,
        parent_id: Some(1),
        locator: ResourceLocator::NativePath(root.join("item").to_string_lossy().into_owned()),
        name: "item".into(),
        kind: NodeKind::File,
        subtree_bytes: bytes,
        direct_bytes: bytes,
        size_known: true,
        files: 1,
        directories: 0,
        modified_unix_seconds: None,
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    };
    DiskGraph {
        snapshot: DiskSnapshot {
            id: id.into(),
            root: locator,
            volume_id: Some(volume_id),
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
        nodes: vec![directory, file],
        evidence: vec![],
    }
}

#[cfg(unix)]
fn offline_root() -> &'static Path {
    Path::new("/legacy-native/offline")
}
#[cfg(windows)]
fn offline_root() -> &'static Path {
    Path::new(r"C:\legacy_native\offline")
}
#[cfg(unix)]
fn raw_names() -> [OsString; 2] {
    [
        OsString::from_vec(b"root-\xff".to_vec()),
        OsString::from_vec(b"root-\xfe".to_vec()),
    ]
}
#[cfg(windows)]
fn raw_names() -> [OsString; 2] {
    [0xd800, 0xd801].map(|unit| {
        let mut units: Vec<u16> = "root-".encode_utf16().collect();
        units.push(unit);
        OsString::from_wide(&units)
    })
}
