//! 准备预算长短合法定位的整次 Rust requested 分配观测；来源：D42 EC-04。
//! 当前只冻结原生基线，不凭估算宣称 8KiB 差值门禁；不计 SQLite C、I/O 或 RSS。
#![cfg(target_os = "linux")]
mod process_preparation_budget {
    pub(super) mod fixture;
}
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use diskgraph_store::{JobState, StoreError};
use process_preparation_budget::fixture::Fixture;
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
#[test]
fn records_whole_call_preparation_cost_for_real_short_and_long_locators() {
    const NAME: &str = "records_whole_call_preparation_cost_for_real_short_and_long_locators";
    const CHILD: &str = "DISKGRAPH_PROCESS_PREPARATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([NAME, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
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
        return;
    }
    let fixtures = [Fixture::new(false), Fixture::new(true)];
    let mut tiny_bytes = [0usize; 2];
    for (index, f) in fixtures.iter().enumerate() {
        let job = f.enqueue(true);
        let (result, bytes, elapsed) =
            measured(|| f.engine.run_job_strict(&job.job_id, "measured-owner"));
        tiny_bytes[index] = bytes;
        println!(
            "long={}, raw_root_bytes={}, metadata_limit=1, whole_call_rust_requested={}, elapsed={elapsed:?}, result={result:?}",
            index == 1,
            f.root.as_os_str().len(),
            bytes
        );
        assert!(
            elapsed < Duration::from_secs(15),
            "deadline failure cannot substitute for raw rejection"
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::BudgetExceeded))
                    | Err(EngineError::Store(StoreError::BudgetExceeded))
            ),
            "{result:?}"
        );
        f.assert_failed(&job.job_id);
        // 足额真实执行正控；持有本文件，fixture/入队/打印都在测量窗外。
        let held = std::fs::File::open(f.root.join("target")).unwrap();
        let job = f.enqueue(false);
        let (result, bytes, elapsed) =
            measured(|| f.engine.run_job_strict(&job.job_id, "measured-owner"));
        println!(
            "long={}, raw_root_bytes={}, metadata_limit=default, whole_call_rust_requested={}, elapsed={elapsed:?}, result={result:?}",
            index == 1,
            f.root.as_os_str().len(),
            bytes
        );
        assert_eq!(result.unwrap().state, JobState::Completed);
        drop(held);
    }
    println!(
        "paired_tiny_whole_call_long_minus_short={}",
        tiny_bytes[1] as i128 - tiny_bytes[0] as i128
    );
    // 仅记录实际基线；准备前准入另由 request-local CAPTURE 未消费的精确行为断言证明。
}
