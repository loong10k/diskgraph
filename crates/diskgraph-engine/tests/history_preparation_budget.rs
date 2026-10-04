//! 整个公开历史请求的必要目标准入；来源：OpenSpec Q-08。
//! Rust requested allocation 不代表 SQLite C、I/O 或 RSS；合法导入不在测量窗口内。

mod history_preparation_budget {
    pub(super) mod callback_authorizer;
    pub(super) mod fixture;
}
use diskgraph_core::{BusinessError, QueryBudget};
use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use history_preparation_budget::fixture::Fixture;
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static MEASURING: AtomicBool = AtomicBool::new(false);
static REQUESTED_BYTES: AtomicUsize = AtomicUsize::new(0);

/// 独占测试子进程的 System 透明计量器；来源：原生 Rust Q-08 请求成本回归。
struct MeasuredAllocator;
// SAFETY: 原指针、布局、大小均不变地交给 System；原子计数本身不分配。
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(size, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
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
    const CHILD: &str = "DISKGRAPH_HISTORY_PREPARATION_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(name) {
        work();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    eprintln!("{stdout}");
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("test result: ok. 1 passed"));
}

fn request(
    fixture: &Fixture,
    method: &str,
    raw_bytes: usize,
) -> Result<serde_json::Value, EngineError> {
    let budget = QueryBudget {
        max_response_bytes: raw_bytes,
        deadline_ms: 1000,
        ..QueryBudget::default()
    };
    let deadline = Instant::now() + Duration::from_millis(budget.deadline_ms);
    let result = match method {
        "compare" => fixture
            .engine
            .compare_revisions_until(
                "left",
                "right",
                0,
                budget,
                &fixture.principal,
                &fixture.policy,
                deadline,
            )
            .map(|report| report.to_json(None)),
        "growth" => fixture
            .engine
            .growth_between_until(
                "left",
                "right",
                Path::new("item"),
                budget,
                &fixture.principal,
                &fixture.policy,
                deadline,
            )
            .map(|value| {
                value.map_or(serde_json::Value::Null, |growth| {
                    serde_json::json!({"delta":growth.delta_bytes.to_string(),
                    "before":growth.before.subtree_bytes,"after":growth.after.subtree_bytes})
                })
            }),
        "changes" => fixture.engine.revision_changes_until(
            "left",
            "right",
            budget,
            &fixture.principal,
            &fixture.policy,
            deadline,
        ),
        _ => panic!("unknown test method"),
    };
    if matches!(
        result,
        Err(EngineError::Store(StoreError::BudgetExceeded))
            | Err(EngineError::Business(BusinessError::BudgetExceeded))
    ) {
        assert!(
            Instant::now() < deadline,
            "deadline failure cannot substitute for raw admission"
        );
    }
    result
}

fn assert_success(method: &str, result: Result<serde_json::Value, EngineError>) {
    let value = result.unwrap();
    match method {
        "compare" => {
            assert_eq!(value["complete"], true);
            assert_eq!(value["left"]["revision_id"], "left");
            assert_eq!(value["right"]["revision_id"], "right");
            assert_eq!(value["left"]["nodes"], 2);
            assert_eq!(value["right"]["nodes"], 2);
            assert_eq!(value["summary"]["different"], 1);
            assert_eq!(value["entries"], 1);
            assert_eq!(value["rows"][0]["path"], "item");
            assert_eq!(value["rows"][0]["left_bytes"], 10);
            assert_eq!(value["rows"][0]["right_bytes"], 15);
        }
        "growth" => assert_eq!(
            value,
            serde_json::json!({"delta":"5","before":10,"after":15})
        ),
        "changes" => {
            assert_eq!(value["complete"], true);
            assert_eq!(value["incompatible"], serde_json::Value::Null);
            assert_eq!(value["size_changed"], 1);
        }
        _ => panic!("unknown test method"),
    }
}

fn assert_raw_refusal(result: Result<serde_json::Value, EngineError>) {
    assert!(
        matches!(
            result,
            Err(EngineError::Store(StoreError::BudgetExceeded))
                | Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "required raw header must be refused, not another error or partial report: {result:?}"
    );
}

fn oversized_header(method: &str, large_left: bool) {
    // 左右侧独立覆盖；短 revision、真实 owner/grant 和普通节点排除无关错误。
    let (left, right) = if large_left {
        (2 << 20, 8)
    } else {
        (8, 2 << 20)
    };
    let fixture = Fixture::new(left, right);
    let (result, allocated) = measured(|| request(&fixture, method, 64 << 10));
    println!(
        "method={method}, left_id={left}, right_id={right}, whole_request_rust_bytes={allocated}, result={result:?}"
    );
    assert!(
        allocated < 1 << 20,
        "oversized revision target was owned before raw admission: {allocated}"
    );
    assert_raw_refusal(result);
}

#[test]
fn compare_admits_left_revision_target_before_owning_it() {
    isolated(
        "compare_admits_left_revision_target_before_owning_it",
        || oversized_header("compare", true),
    );
}
#[test]
fn compare_admits_right_revision_target_before_owning_it() {
    isolated(
        "compare_admits_right_revision_target_before_owning_it",
        || oversized_header("compare", false),
    );
}
#[test]
fn growth_admits_left_revision_target_before_owning_it() {
    isolated(
        "growth_admits_left_revision_target_before_owning_it",
        || oversized_header("growth", true),
    );
}
#[test]
fn growth_admits_right_revision_target_before_owning_it() {
    isolated(
        "growth_admits_right_revision_target_before_owning_it",
        || oversized_header("growth", false),
    );
}
#[test]
fn changes_admits_left_revision_target_before_owning_it() {
    isolated(
        "changes_admits_left_revision_target_before_owning_it",
        || oversized_header("changes", true),
    );
}
#[test]
fn changes_admits_right_revision_target_before_owning_it() {
    isolated(
        "changes_admits_right_revision_target_before_owning_it",
        || oversized_header("changes", false),
    );
}
#[test]
fn ordinary_and_adequately_budgeted_headers_keep_real_history_results() {
    isolated(
        "ordinary_and_adequately_budgeted_headers_keep_real_history_results",
        || {
            for (length, budget) in [(8, 64 << 10), (2 << 20, 32 << 20)] {
                let fixture = Fixture::new(length, length);
                for method in ["compare", "growth", "changes"] {
                    let (result, allocated) = measured(|| request(&fixture, method, budget));
                    println!(
                        "method={method}, positive_id={length}, budget={budget}, whole_request_rust_bytes={allocated}"
                    );
                    assert_success(method, result);
                }
            }
        },
    );
}
#[test]
fn both_headers_and_snapshot_preparation_share_the_original_raw_balance() {
    isolated(
        "both_headers_and_snapshot_preparation_share_the_original_raw_balance",
        || {
            for method in ["compare", "growth", "changes"] {
                // compare 只需节点；growth/changes 还读取两份 snapshot JSON，必要读取不能重置额度。
                let length = if method == "compare" {
                    40 << 10
                } else {
                    20 << 10
                };
                let single = Fixture::new(length, 8);
                assert_success(method, request(&single, method, 64 << 10));
                let both = Fixture::new(length, length);
                assert_raw_refusal(request(&both, method, 64 << 10));
                assert_success(method, request(&both, method, 256 << 10));
            }
        },
    );
}

#[test]
fn actual_permission_denial_precedes_the_oversized_target() {
    isolated(
        "actual_permission_denial_precedes_the_oversized_target",
        || {
            let mut fixture = Fixture::new(2 << 20, 8);
            fixture.policy = diskgraph_core::PolicyAuthorizer::new(1);
            for method in ["compare", "growth", "changes"] {
                let (result, allocated) = measured(|| request(&fixture, method, 64 << 10));
                assert!(
                    allocated < 1 << 20,
                    "unauthorized target was allocated: {allocated}"
                );
                assert!(
                    matches!(
                        result,
                        Err(EngineError::Business(BusinessError::PermissionDenied))
                    ),
                    "target budget replaced real authorization: {result:?}"
                );
            }
        },
    );
}

#[test]
fn terminal_revocation_precedes_an_admitted_owner_target_budget_failure() {
    use history_preparation_budget::callback_authorizer::CallbackAuthorizer;
    let fixture = Fixture::new(2 << 20, 8);
    let path = fixture.engine.data_dir().join("diskgraph-control.sqlite");
    let scope = fixture
        .engine
        .authorize_revision(None, "left", &fixture.principal, &fixture.policy)
        .unwrap();
    let revoke_scope = scope.clone();
    let policy = CallbackAuthorizer::new(fixture.policy.clone(), move |call| {
        if call == 3 {
            diskgraph_store::ControlStore::open(&path)
                .unwrap()
                .revoke_scope(&revoke_scope)
                .unwrap();
        }
    });
    let result = fixture.engine.revision_changes_until(
        "left",
        "right",
        QueryBudget::default(),
        &fixture.principal,
        &policy,
        Instant::now() + Duration::from_secs(1),
    );
    assert!(
        policy.calls.get() >= 3,
        "real terminal authorization was not reached"
    );
    assert!(fixture.engine.scope(&scope).unwrap().revoked);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "terminal denial was hidden by raw failure: {result:?}"
    );
}
