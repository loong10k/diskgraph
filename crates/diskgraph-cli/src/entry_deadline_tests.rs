//! CLI 启动准入与查询期限的真实隔离数据库回归；来源：Q-09 dispatch 绝对期限合同。
use clap::Parser;
use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, NodeKind, PrincipalId, QueryBudget, ResourceLocator,
    ScanCoverage, ScanSettings,
};
use diskgraph_engine::{Engine, EngineConfig};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

thread_local! {
    static AFTER_STARTUP: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// 参数：无；返回：无，仅测试线程执行一次受控启动延迟，不更改真实 Engine/DB/授权。
pub(super) fn after_startup() {
    let hook = AFTER_STARTUP.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[test]
fn slow_startup_does_not_spend_the_indexed_request_budget() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("registered-root");
    let data = directory.path().join("data");
    std::fs::create_dir(&root).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    // 仅在隔离库建立真实 staging/revision；该测试验证查询，不声明执行过物理扫描。
    let locator = ResourceLocator::NativePath(
        engine
            .control_store()
            .unwrap()
            .scope(&scope)
            .unwrap()
            .root
            .display,
    );
    let graph = DiskGraph {
        snapshot: DiskSnapshot {
            id: "startup-budget-snapshot".into(),
            root: locator.clone(),
            volume_id: None,
            captured_at_unix_ms: 1,
            settings: ScanSettings {
                apparent_size: false,
                follow_links: false,
                include_hidden: false,
                one_filesystem: false,
                max_depth: None,
                dedup_hardlinks: true,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes: vec![DiskNode {
            id: 1,
            parent_id: None,
            locator,
            name: "registered-root".into(),
            kind: NodeKind::Directory,
            subtree_bytes: 0,
            direct_bytes: 0,
            size_known: true,
            files: 0,
            directories: 0,
            modified_unix_seconds: None,
            file_identity: None,
            category_hint: None,
            reclaim_hint: None,
            read_error: false,
        }],
        evidence: vec![],
    };
    let mut store =
        diskgraph_store::SqliteSnapshotStore::open(&data.join("diskgraph.sqlite")).unwrap();
    store
        .append_staging_nodes("startup-budget", &graph.nodes)
        .unwrap();
    store
        .publish_revision_owned(
            "startup-budget",
            &graph,
            "startup-budget-revision",
            1,
            Some((engine.server_id().unwrap().as_str(), scope.as_str())),
        )
        .unwrap();
    drop(store);
    drop(engine);
    let cli = crate::cli::Cli::try_parse_from([
        "diskgraph",
        "--data-dir",
        data.to_str().unwrap(),
        "--json",
        "candidates",
        "--scope",
        scope.as_str(),
        "--target-bytes",
        "1",
    ])
    .unwrap();
    let observed = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&observed);
    AFTER_STARTUP.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            std::thread::sleep(Duration::from_millis(
                QueryBudget::default().deadline_ms + 20,
            ));
            mark.store(true, Ordering::SeqCst);
        }));
    });
    let result = super::run(cli);
    let leftover = AFTER_STARTUP.with(|slot| slot.borrow_mut().take());
    assert!(
        leftover.is_none() && observed.load(Ordering::SeqCst),
        "actual startup seam was not reached"
    );
    assert!(
        result.is_ok(),
        "indexed query lost its request budget during startup: {result:?}"
    );
}
