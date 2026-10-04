//! 完整公开关系/树/候选请求的准备成本回归；来源：OpenSpec Q-08。
//! 合法导入不在计量窗口，Rust requested allocation 不等于 RSS/SQLite C/I/O。
mod relation_preparation_budget {
    pub(super) mod fixture;
}
use diskgraph_core::{BusinessError, QueryBudget};
use diskgraph_engine::EngineError;
use diskgraph_store::{SqliteSnapshotStore, StoreError};
use relation_preparation_budget::fixture::Fixture;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
#[path = "history_preparation_budget/callback_authorizer.rs"]
mod callback_authorizer;
static MEASURING: AtomicBool = AtomicBool::new(false);
static REQUESTED_BYTES: AtomicUsize = AtomicUsize::new(0);

/// 独占测试子进程的 System 透明计量器；来源：Q-08 请求成本回归。
struct MeasuredAllocator;
// SAFETY: 指针、布局和长度原样交给 System；原子计数不进行分配。
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(pointer, layout, size) };
        if !p.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(size, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}
#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;
fn measured<T>(work: impl FnOnce() -> T) -> (T, usize) {
    REQUESTED_BYTES.store(0, Ordering::Relaxed);
    MEASURING.store(true, Ordering::Relaxed);
    let result = work();
    MEASURING.store(false, Ordering::Relaxed);
    (result, REQUESTED_BYTES.load(Ordering::Relaxed))
}
fn isolated(name: &str, work: impl FnOnce()) {
    const KEY: &str = "DISKGRAPH_RELATION_PREPARATION_CHILD";
    if std::env::var(KEY).ok().as_deref() == Some(name) {
        work();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(KEY, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("{stdout}");
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("test result: ok. 1 passed"));
}
fn request(f: &Fixture, method: &str, bytes: usize) -> Result<serde_json::Value, EngineError> {
    let budget = QueryBudget {
        max_response_bytes: bytes,
        deadline_ms: 1000,
        ..QueryBudget::default()
    };
    let deadline = Instant::now() + Duration::from_millis(1000);
    let e = &f.base.engine;
    let p = &f.base.principal;
    let a = &f.base.policy;
    let result = match method {
        "related" => e.related_bounded_until(
            "selected", "entity", None, false, None, 1, budget, p, a, deadline,
        ),
        "explain" => e.explain_bounded_until("selected", "entity", None, 1, budget, p, a, deadline),
        "impact" => e
            .revision_impact_until("selected", "entity", budget, p, a, deadline)
            .map(|v| serde_json::to_value(v).unwrap()),
        "candidate" => e
            .review_candidates_until("selected", 0, budget, p, a, deadline)
            .map(|v| serde_json::to_value(v).unwrap()),
        "tree" => e
            .tree_view_until(&f.scope, "selected", p, a, 1, 0, budget, deadline)
            .map(|v| v.root),
        "trusted_tree" => e
            .tree_view_bounded_until("selected", 1, 0, budget, deadline)
            .map(|v| v.root),
        "store_candidate" => SqliteSnapshotStore::open_reader_until(
            &f.data_dir().join("diskgraph.sqlite"),
            deadline,
            None,
        )
        .and_then(|r| r.candidate_selection_for_revision_until("selected", 0, budget, deadline))
        .map_err(EngineError::from)
        .map(|v| serde_json::to_value(v).unwrap()),
        _ => panic!("invalid fixture method"),
    };
    assert!(
        Instant::now() < deadline,
        "deadline must not substitute for raw admission"
    );
    result
}
fn oversized(method: &str) {
    let f = Fixture::new(2 << 20);
    let (result, allocated) = measured(|| request(&f, method, 64 << 10));
    println!("method={method}, whole_request_rust_bytes={allocated}, result={result:?}");
    assert!(
        allocated < 1 << 20,
        "target owned before admission: {allocated}"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Store(StoreError::BudgetExceeded))
                | Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "wrong preparation outcome: {result:?}"
    );
}
macro_rules! regression {
    ($name:ident,$method:literal) => {
        #[test]
        fn $name() {
            isolated(stringify!($name), || oversized($method));
        }
    };
}
regression!(
    related_admits_the_required_revision_target_before_owning_it,
    "related"
);
regression!(
    explain_admits_the_required_revision_target_before_owning_it,
    "explain"
);
regression!(
    impact_admits_the_required_revision_target_before_owning_it,
    "impact"
);
regression!(
    candidate_admits_the_required_revision_target_before_owning_it,
    "candidate"
);
regression!(
    tree_admits_the_required_revision_target_before_owning_it,
    "tree"
);
regression!(
    trusted_tree_admits_the_required_revision_target_before_owning_it,
    "trusted_tree"
);
regression!(
    store_candidate_admits_the_required_revision_target_before_owning_it,
    "store_candidate"
);
#[test]
fn ordinary_and_adequately_budgeted_targets_keep_real_results() {
    for (size, bytes) in [(8, 64 << 10), (2 << 20, 16 << 20)] {
        let f = Fixture::new(size);
        for method in [
            "related",
            "explain",
            "impact",
            "candidate",
            "tree",
            "trusted_tree",
            "store_candidate",
        ] {
            let result = request(&f, method, bytes).unwrap();
            match method {
                "related" => {
                    assert_eq!(result["complete"], true);
                    assert_eq!(result["edges"], serde_json::json!([]));
                }
                "explain" => {
                    assert_eq!(result["complete"], true);
                    assert_eq!(result["entity"]["entity_id"], "entity");
                }
                "impact" => {
                    assert_eq!(result["entries"], serde_json::json!([]));
                    assert_eq!(result["truncated"], serde_json::Value::Null);
                }
                "candidate" | "store_candidate" => {
                    assert_eq!(result["complete"], true);
                    assert_eq!(result["coverage_observed"], true);
                }
                "tree" | "trusted_tree" => {
                    assert_eq!(result["name"], "root");
                    assert_eq!(result["children"][0]["name"], "item");
                }
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn every_consumer_keeps_the_remaining_preparation_balance() {
    let f = Fixture::with_edge(31 << 10, 4096);
    let db = rusqlite::Connection::open_with_flags(
        f.data_dir().join("diskgraph.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let snapshot: String = db
        .query_row(
            "SELECT snapshot_id FROM graph_revisions WHERE revision_id='selected'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let entity_bytes: i64 = db.query_row("SELECT length(CAST(entity_json AS BLOB)) FROM entities WHERE snapshot_id=?1 AND entity_id='entity'", [&snapshot], |r| r.get(0)).unwrap();
    let edge_bytes: i64 = db.query_row("SELECT length(CAST(edge_json AS BLOB)) FROM relations WHERE snapshot_id=?1 AND edge_id='edge'", [&snapshot], |r| r.get(0)).unwrap();
    let header_bytes: i64 = db
        .query_row(
            "SELECT length(CAST(snapshot_json AS BLOB)) FROM snapshots WHERE id=?1",
            [&snapshot],
            |r| r.get(0),
        )
        .unwrap();
    let reader = SqliteSnapshotStore::open_reader_until(
        &f.data_dir().join("diskgraph.sqlite"),
        Instant::now() + Duration::from_secs(2),
        None,
    )
    .unwrap();
    // 以真实公开读取观察根字段成本，不把 SQL 页缓存计入 Rust 原始额度。
    let mut ledger = diskgraph_core::QueryReadBudget::new(
        QueryBudget::default(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    reader
        .root_node_with_budget(&snapshot, &mut ledger)
        .unwrap()
        .unwrap();
    let root_bytes = QueryBudget::default().max_response_bytes - ledger.remaining_raw_bytes();
    for (method, consumer_bytes) in [
        ("related", usize::try_from(edge_bytes).unwrap()),
        ("explain", usize::try_from(entity_bytes).unwrap()),
        ("impact", usize::try_from(edge_bytes).unwrap()),
        ("candidate", usize::try_from(header_bytes).unwrap()),
        ("tree", root_bytes),
        ("trusted_tree", root_bytes),
        ("store_candidate", usize::try_from(header_bytes).unwrap()),
    ] {
        let allowance = snapshot.len() + consumer_bytes - 1;
        assert!(snapshot.len() < allowance && consumer_bytes < allowance);
        let result = request(&f, method, allowance);
        println!(
            "cumulative method={method}, target_bytes={}, consumer_bytes={consumer_bytes}, allowance={allowance}, result={result:?}",
            snapshot.len()
        );
        if matches!(method, "related" | "impact") {
            let value = result.unwrap();
            assert_eq!(value["truncated"], "byte_limit");
            assert!(
                value[if method == "impact" {
                    "entries"
                } else {
                    "edges"
                }]
                .as_array()
                .unwrap()
                .is_empty()
            );
            if method == "related" {
                assert_eq!(value["complete"], false);
            }
        } else {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Store(StoreError::BudgetExceeded))
                        | Err(EngineError::Business(BusinessError::BudgetExceeded))
                ),
                "new consumer balance: {method}: {result:?}"
            );
        }
        let paired = request(&f, method, 256 << 10).unwrap();
        match method {
            "related" | "explain" => {
                assert_eq!(paired["complete"], true);
                assert_eq!(paired["edges"].as_array().unwrap().len(), 1);
            }
            "impact" => {
                assert_eq!(paired["truncated"], serde_json::Value::Null);
                assert_eq!(paired["entries"].as_array().unwrap().len(), 1);
            }
            "candidate" | "store_candidate" => {
                assert_eq!(paired["complete"], true);
                assert_eq!(paired["coverage_observed"], true);
            }
            "tree" | "trusted_tree" => assert_eq!(paired["children"][0]["name"], "item"),
            _ => unreachable!(),
        }
    }
}

#[test]
fn target_budget_failure_still_checks_terminal_scope_revocation() {
    for method in ["related", "explain", "impact", "candidate", "tree"] {
        let f = Fixture::new(2 << 20);
        let data = f.data_dir();
        let scope = f.scope.clone();
        let auth =
            callback_authorizer::CallbackAuthorizer::new(f.base.policy.clone(), move |call| {
                if call == 2 {
                    diskgraph_store::ControlStore::open(&data.join("diskgraph-control.sqlite"))
                        .unwrap()
                        .revoke_scope(&scope)
                        .unwrap();
                }
            });
        let budget = QueryBudget {
            max_response_bytes: 64 << 10,
            deadline_ms: 1000,
            ..QueryBudget::default()
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        let e = &f.base.engine;
        let p = &f.base.principal;
        let result = match method {
            "related" => e
                .related_bounded_until(
                    "selected", "entity", None, true, None, 1, budget, p, &auth, deadline,
                )
                .map(|_| ()),
            "explain" => e
                .explain_bounded_until("selected", "entity", None, 1, budget, p, &auth, deadline)
                .map(|_| ()),
            "impact" => e
                .revision_impact_until("selected", "entity", budget, p, &auth, deadline)
                .map(|_| ()),
            "candidate" => e
                .review_candidates_until("selected", 1, budget, p, &auth, deadline)
                .map(|_| ()),
            "tree" => e
                .tree_view_until(&f.scope, "selected", p, &auth, 1, 0, budget, deadline)
                .map(|_| ()),
            _ => unreachable!(),
        };
        assert!(auth.calls.get() >= 2);
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "target failure escaped terminal denial: {method}: {result:?}"
        );
    }
}

#[test]
fn tree_scope_assertion_denies_before_owning_a_large_target() {
    isolated(
        "tree_scope_assertion_denies_before_owning_a_large_target",
        || {
            let f = Fixture::new(2 << 20);
            let wrong = diskgraph_core::ScopeId::new("different-actual-scope").unwrap();
            let budget = QueryBudget {
                deadline_ms: 1000,
                ..QueryBudget::default()
            };
            let (result, allocated) = measured(|| {
                f.base.engine.tree_view_until(
                    &wrong,
                    "selected",
                    &f.base.principal,
                    &f.base.policy,
                    1,
                    0,
                    budget,
                    Instant::now() + Duration::from_secs(1),
                )
            });
            assert!(allocated < 1 << 20);
            assert!(matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ));
        },
    );
}
