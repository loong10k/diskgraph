//! 旧归属回填只使用可证明无损的身份；所有数据库均为隔离夹具，不启动扫描。
use diskgraph_core::{
    DiskGraph, DiskSnapshot, Locator, ResourceLocator, ScanCoverage, ScanSettings,
};
use diskgraph_engine::{Engine, EngineConfig};
use diskgraph_store::{ControlStore, SqliteSnapshotStore};

fn backfill(roots: &[Locator], legacy_root: ResourceLocator) -> Option<(String, String)> {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let mut control = ControlStore::open(&data.join("diskgraph-control.sqlite")).unwrap();
    control.ensure_server().unwrap();
    for root in roots {
        control.register_scope(root, None).unwrap();
    }
    drop(control);
    let graph_path = data.join("diskgraph.sqlite");
    let mut store = SqliteSnapshotStore::open(&graph_path).unwrap();
    let graph = DiskGraph {
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
    };
    store
        .publish_revision("legacy-job", &graph, "legacy-revision", 1)
        .unwrap();
    drop(store);
    let engine = Engine::open(EngineConfig {
        data_dir: data,
        ..EngineConfig::default()
    })
    .unwrap();
    let result = SqliteSnapshotStore::open(&graph_path)
        .unwrap()
        .revision_ownership("legacy-revision")
        .unwrap();
    drop(engine);
    result
}

#[test]
fn exact_unicode_native_root_still_backfills() {
    let path = std::path::Path::new("/isolated/目录");
    assert!(
        backfill(
            &[Locator::from_native_path(path)],
            ResourceLocator::NativePath(path.to_str().unwrap().into())
        )
        .is_some()
    );
}

#[cfg(unix)]
#[test]
fn single_non_utf8_display_alias_must_not_claim_legacy_revision() {
    use std::os::unix::ffi::OsStringExt;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/isolated/r\xff".to_vec()));
    let root = Locator::from_native_path(&path);
    assert!(
        backfill(
            std::slice::from_ref(&root),
            ResourceLocator::NativePath(root.display.clone())
        )
        .is_none(),
        "unique display match is not a lossless scope identity"
    );
}

#[cfg(unix)]
#[test]
fn excluding_lossy_scope_must_not_turn_colliding_unicode_scope_into_unique_match() {
    use std::os::unix::ffi::OsStringExt;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/isolated/r\xff".to_vec()));
    let lossy = Locator::from_native_path(&path);
    let unicode = Locator::from_native_path(std::path::Path::new(&lossy.display));
    assert!(
        backfill(
            &[unicode, lossy.clone()],
            ResourceLocator::NativePath(lossy.display)
        )
        .is_none()
    );
}

#[cfg(windows)]
#[test]
fn single_unpaired_utf16_display_alias_must_not_claim_legacy_revision() {
    use std::os::windows::ffi::OsStringExt;
    let units = [u16::from(b'C'), u16::from(b':'), u16::from(b'\\'), 0xd800];
    let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&units));
    let root = Locator::from_native_path(&path);
    assert!(
        backfill(
            std::slice::from_ref(&root),
            ResourceLocator::NativePath(root.display.clone())
        )
        .is_none()
    );
}
