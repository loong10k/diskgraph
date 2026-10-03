//! 原生平台旧历史元数据夹具与真实 Linux FS 两种独立来源。
use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, Grant, Locator, NodeKind, Permission, PrincipalId,
    QueryBudget, ResourceLocator, ScanCoverage, ScanSettings, ScopeId,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use diskgraph_store::SqliteSnapshotStore;
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::{ffi::OsStringExt, fs::MetadataExt};
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 独占控制库与显式归属的旧历史；来源：Q-04/D34 原始定位元数据夹具。
pub(crate) struct ScopeHistory {
    pub(crate) directory: tempfile::TempDir,
    pub(crate) engine: Engine,
    pub(crate) principal: PrincipalId,
    roots: [PathBuf; 2],
    live_roots: bool,
    volume_id: String,
    pub(crate) scopes: [ScopeId; 2],
}

impl ScopeHistory {
    /// 构造本平台编码的离线旧记录；参数：无；返回：已验证双侧授权的元数据夹具。
    pub(crate) fn new() -> Self {
        Self::build(false)
    }

    /// 使用当前 Linux 的真实原始目录；参数：无；返回：有真实 FS 前提的夹具。
    #[cfg(target_os = "linux")]
    pub(crate) fn new_live_linux() -> Self {
        Self::build(true)
    }

    fn build(live_roots: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        // 离线分支是本平台合法旧记录语义的合成元数据；不声称执行过真实迁移。
        let parent = if live_roots {
            directory.path()
        } else {
            offline_root()
        };
        let roots = raw_names().map(|name| parent.join(name));
        let roots = if live_roots {
            // 唯一传 true 的 new_live_linux 构造器已由 target_os 门禁限定。
            for root in &roots {
                std::fs::create_dir(root).unwrap();
            }
            roots.map(|root| root.canonicalize().unwrap())
        } else {
            roots
        };
        let volume_id = if live_roots {
            #[cfg(unix)]
            {
                std::fs::metadata(&roots[0]).unwrap().dev().to_string()
            }
            #[cfg(windows)]
            {
                unreachable!("live raw directories are Linux-only")
            }
        } else {
            "2049".to_owned()
        };
        assert_ne!(roots[0], roots[1]);
        assert_eq!(roots[0].to_string_lossy(), roots[1].to_string_lossy());
        let config = EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        };
        let engine = Engine::open(config.clone()).unwrap();
        let principal = PrincipalId::new("scope-history-reader").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scopes = if live_roots {
            roots.each_ref().map(|root| {
                engine
                    .register_scope(root, &principal, &engine.policy_authorizer().unwrap())
                    .unwrap()
            })
        } else {
            // 公开可信 ControlStore API 只持久保存原始定位，不触发当前文件系统访问。
            let mut control = engine.control_store().unwrap();
            let version = control.policy_version().unwrap();
            roots.each_ref().map(|root| {
                let locator = Locator::from_native_path(root);
                let scope = control.register_scope(&locator, Some(&volume_id)).unwrap();
                control
                    .upsert_grant(&Grant {
                        principal: principal.clone(),
                        permission: Permission::MetadataRead,
                        scope: scope.clone(),
                        policy_version: version,
                    })
                    .unwrap();
                assert_eq!(
                    control.register_scope(&locator, Some(&volume_id)).unwrap(),
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
        assert!(first.volume_id.is_some());
        let mut fixture = Self {
            directory,
            engine,
            principal,
            roots,
            live_roots,
            volume_id,
            scopes,
        };
        fixture.publish("before", 0, 100, 1, None);
        fixture.publish("after", 1, 200, 2, None);
        // 旧/import 显式归属在启动 backfill 后仍须保留，不由相同显示文本重新绑定。
        fixture.engine = Engine::open(config).unwrap();
        let policy = fixture.engine.policy_authorizer().unwrap();
        for (revision, scope) in [
            ("before", &fixture.scopes[0]),
            ("after", &fixture.scopes[1]),
        ] {
            assert_eq!(
                fixture
                    .engine
                    .authorize_revision(Some(scope), revision, &fixture.principal, &policy)
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

    pub(crate) fn publish(
        &self,
        id: &str,
        side: usize,
        bytes: u64,
        time: u64,
        server: Option<&str>,
    ) {
        let graph = if self.live_roots {
            legacy_graph(id, &self.roots[side], bytes, time)
        } else {
            stored_graph(id, &self.roots[side], bytes, time, self.volume_id.clone())
        };
        let mut store =
            SqliteSnapshotStore::open(&self.directory.path().join("data/diskgraph.sqlite"))
                .unwrap();
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

    pub(crate) fn growth(
        &self,
        before: &str,
        after: &str,
    ) -> Result<Option<diskgraph_engine::RevisionGrowth>, EngineError> {
        self.engine.growth_between_until(
            before,
            after,
            Path::new("item"),
            QueryBudget::default(),
            &self.principal,
            &self.engine.policy_authorizer().unwrap(),
            Instant::now() + Duration::from_secs(30),
        )
    }
}

pub(crate) fn legacy_graph(id: &str, root: &Path, bytes: u64, time: u64) -> DiskGraph {
    std::fs::write(root.join("item"), vec![1u8; bytes as usize]).unwrap();
    let measured = std::fs::metadata(root.join("item")).unwrap().len();
    assert_eq!(measured, bytes);
    #[cfg(unix)]
    let volume = std::fs::metadata(root).unwrap().dev().to_string();
    #[cfg(windows)]
    let volume = String::new();
    let mut graph = stored_graph(id, root, bytes, time, volume);
    if cfg!(windows) {
        // 此辅助夹具只测通用跨根比较，不伪造当前 Windows 的真实卷观测。
        graph.snapshot.volume_id = None;
    }
    graph
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
