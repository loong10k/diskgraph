//! Process 入队必要投影的整次分配回归；来源：D42 EC-04，不读取或修改生产实现。
//! System requested 字节包括整个公开调用，不代表 SQLite C、I/O、RSS 或峰值；512KiB 只排除单次 2MiB 拷贝。
mod process_entry_scope_budget {
    pub(super) mod fixture;
}
use diskgraph_core::{BusinessError, Permission, QueryBudget, QueryReadBudget, TruncationReason};
use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use process_entry_scope_budget::fixture::Fixture;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static MEASURING: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
/// 独占子进程的透明分配计量，不接管生产分配器；来源：原生 Rust whole-call 验收。
struct MeasuredAllocator;
// SAFETY: System 原样接收指针/布局/尺寸，计数不分配，不读取无效内存。
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED.fetch_add(size, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;
fn measured<T>(work: impl FnOnce() -> T) -> (T, usize, Duration) {
    REQUESTED.store(0, Ordering::Relaxed);
    let start = Instant::now();
    MEASURING.store(true, Ordering::Relaxed);
    let value = work();
    MEASURING.store(false, Ordering::Relaxed);
    (value, REQUESTED.load(Ordering::Relaxed), start.elapsed())
}

const GIANT: usize = 2 * 1024 * 1024;
const NO_GIANT_COPY: usize = 512 * 1024;

fn child(name: &str) -> bool {
    const KEY: &str = "DISKGRAPH_PROCESS_ENTRY_SCOPE_CHILD";
    if std::env::var(KEY).ok().as_deref() == Some(name) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(KEY, name)
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
    false
}
fn full_entry(f: &Fixture) -> usize {
    let authority = f.authority(vec![Permission::MetadataRead, Permission::IndexWrite]);
    let auth = f.engine.policy_authorizer().unwrap();
    // 注册、索引、构造 token/policy 均在窗外；初始权限、实际 owner、所有准备及入队都在窗内。
    let (result, bytes, elapsed) = measured(|| {
        f.engine
            .process_evidence_scope_with_authority(&f.scope, &f.base, f.node, &authority, &auth)
    });
    println!(
        "fields_each={}, raw_request_limit={}, whole_call_rust_requested={bytes}, elapsed={elapsed:?}, result={result:?}",
        f.field_bytes,
        QueryBudget::default().max_response_bytes
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "a deadline refusal cannot substitute for projection: {elapsed:?}"
    );
    f.assert_platform_result(result);
    bytes
}
fn denied_entry(missing: Permission) {
    let f = Fixture::new(GIANT);
    let capabilities = [Permission::MetadataRead, Permission::IndexWrite]
        .into_iter()
        .filter(|p| p != &missing)
        .collect();
    let authority = f.authority(capabilities);
    let auth = f.engine.policy_authorizer().unwrap();
    let (result, bytes, elapsed) = measured(|| {
        f.engine
            .process_evidence_scope_with_authority(&f.scope, &f.base, f.node, &authority, &auth)
    });
    println!(
        "missing={missing:?}, fields_each={GIANT}, whole_call_rust_requested={bytes}, elapsed={elapsed:?}, result={result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "original entry deadline exhausted: {elapsed:?}"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "missing original capability: {result:?}"
    );
    f.assert_no_queue();
    assert!(
        bytes < NO_GIANT_COPY,
        "denial copied unnecessary scope fields: {bytes} requested bytes"
    );
}

#[test]
fn ordinary_scope_entry_preserves_real_platform_qualification() {
    if !child("ordinary_scope_entry_preserves_real_platform_qualification") {
        return;
    }
    for size in [0, 4096] {
        let f = Fixture::new(size);
        let bytes = full_entry(&f);
        assert!(bytes < NO_GIANT_COPY, "ordinary entry allocation: {bytes}");
    }
}
#[test]
fn giant_optional_scope_fields_are_not_owned_by_public_process_entry() {
    if !child("giant_optional_scope_fields_are_not_owned_by_public_process_entry") {
        return;
    }
    let f = Fixture::new(GIANT);
    let bytes = full_entry(&f);
    assert!(
        bytes < NO_GIANT_COPY,
        "entry copied unnecessary display/volume fields: {bytes} requested bytes"
    );
}
#[test]
fn missing_metadata_capability_denies_before_large_scope_ownership() {
    if child("missing_metadata_capability_denies_before_large_scope_ownership") {
        denied_entry(Permission::MetadataRead);
    }
}
#[test]
fn missing_index_capability_denies_before_large_scope_ownership() {
    if child("missing_index_capability_denies_before_large_scope_ownership") {
        denied_entry(Permission::IndexWrite);
    }
}
#[test]
fn full_scope_projection_with_sufficient_budget_proves_large_registration_is_legal() {
    if !child("full_scope_projection_with_sufficient_budget_proves_large_registration_is_legal") {
        return;
    }
    let f = Fixture::new(GIANT);
    let control = f.engine.control_store().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut small = QueryReadBudget::new(QueryBudget::default(), deadline).unwrap();
    let rejected = control.scope_with_admission(&f.scope, &mut |raw, _, _| {
        if small.admit(0, 0, usize::try_from(raw).unwrap()) {
            Ok(())
        } else {
            Err(StoreError::BudgetExceeded)
        }
    });
    assert!(matches!(rejected, Err(StoreError::BudgetExceeded)));
    assert_eq!(small.stopped(), Some(TruncationReason::ByteLimit));
    assert!(Instant::now() < deadline);
    // 这是完整 scope 投影正控；公开 entry 没有客户端预算参数，也不需要拥有这些显示字段。
    let large_limit = 32 * 1024 * 1024;
    let mut large = QueryReadBudget::new(
        QueryBudget {
            max_response_bytes: large_limit,
            ..QueryBudget::default()
        },
        deadline,
    )
    .unwrap();
    let saved = control
        .scope_with_admission(&f.scope, &mut |raw, _, _| {
            if large.admit(0, 0, usize::try_from(raw).unwrap()) {
                Ok(())
            } else {
                Err(StoreError::BudgetExceeded)
            }
        })
        .unwrap();
    assert_eq!(saved.scope_id, f.scope);
    assert_eq!(saved.root.display.len(), GIANT);
    assert!(saved.root.display.bytes().all(|byte| byte == b'd'));
    assert_eq!(saved.volume_id.as_deref().unwrap().len(), GIANT);
    assert!(
        saved
            .volume_id
            .as_deref()
            .unwrap()
            .bytes()
            .all(|byte| byte == b'v')
    );
    assert!(
        saved
            .root
            .to_native_path()
            .unwrap()
            .join("target")
            .is_file()
    );
    assert!(!saved.revoked);
    let charged = large_limit - large.remaining_raw_bytes();
    assert!(charged >= 2 * GIANT);
    assert_eq!(large.stopped(), None);
    println!("full_scope_projection_raw_admitted={charged}, sufficient_limit={large_limit}");
}
