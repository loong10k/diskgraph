//! 独占子进程计量真实 TUI 读取的 Rust 请求分配；来源：OpenSpec Q-08 / 13.6。
//! 不计量 SQLite C 分配、磁盘读取量或 RSS，合法观测准备发生在计量窗口之外。

use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::test_fixture::TuiFixture;
use super::{PAGE_SIZE, load_layer};
use crate::tui_frame_reader::TuiFrameReader;

static MEASURING: AtomicBool = AtomicBool::new(false);
static REQUESTED_BYTES: AtomicUsize = AtomicUsize::new(0);

/// 原样转发 System 的测试分配器，仅独占子进程的读取窗口计费。
/// 来源：DiskGraph 原生 Rust Q-08 成本回归；无 Java 对应实现。
struct MeasuredAllocator;

// SAFETY: 布局、指针和尺寸未经修改转发 System；计数仅使用不分配内存的原子操作。
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

fn isolated(test_name: &str, work: impl FnOnce()) {
    const CHILD: &str = "DISKGRAPH_TUI_INPUT_ALLOCATION_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(test_name) {
        work();
        return;
    }
    // 父进程永不启用计量，子进程只执行一个真实测试，排除并发测试分配干扰。
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test_name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, test_name)
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

fn navigation_cost(known_size: bool) {
    let fixture = TuiFixture::new(2 << 20, known_size);
    let request = fixture.request();
    let (result, allocated) = measured(|| load_layer(&request, 1));
    println!("known_size={known_size}, navigation_requested_rust_bytes={allocated}");
    assert!(
        allocated < 1 << 20,
        "a small navigation page owned unneeded metadata: {allocated} requested Rust bytes"
    );
    match result {
        Ok(layer) => {
            assert_eq!(layer.name, "root");
            assert_eq!(layer.children.len(), 1);
            assert_eq!(layer.children[0].name, "parent");
            assert_eq!(layer.children[0].size_bytes, 30);
        }
        // 超大旧 JSON 需要借用准入拒绝；不能让普通 deadline 或任意错误充数。
        Err(EngineError::Store(StoreError::BudgetExceeded)) if !known_size => {}
        other => panic!("neither a valid projection nor explicit raw admission: {other:?}"),
    }
}

fn nested_frame_cost(known_size: bool) {
    let fixture = TuiFixture::new(2 << 20, known_size);
    let mut observed = false;
    let result = fixture.engine.with_authorized_revision_reader(
        &fixture.revision,
        &fixture.principal,
        &fixture.policy,
        1000,
        |reader, snapshot, deadline| {
            // cached=0 明确不把实际节点预先缓存，也不省略嵌套 Store 路径。
            let mut reads = TuiFrameReader::new(reader, snapshot, deadline, 0, 0);
            observed = true;
            let (layer, allocated) = measured(|| reads.load_layer(2, PAGE_SIZE));
            println!("known_size={known_size}, nested_requested_rust_bytes={allocated}");
            assert!(
                allocated < 1 << 20,
                "a small nested page owned unneeded metadata: {allocated} requested Rust bytes"
            );
            match layer {
                Some(layer) => {
                    assert_eq!(layer.name, "parent");
                    assert_eq!(layer.children.len(), 2);
                    assert_eq!(layer.children[0].name, "leaf-b");
                    assert_eq!(layer.children[1].name, "leaf-a");
                    assert_eq!(layer.children[0].size_bytes, 20);
                    assert_eq!(layer.children[1].size_bytes, 10);
                    assert!(reads.take_error().is_none());
                }
                None if !known_size => {
                    let failure = reads.take_error();
                    assert!(
                        matches!(
                            reads.truncation_reason,
                            Some(
                                "byte_budget" | "byte_limit" | "raw_byte_budget" | "raw_byte_limit"
                            )
                        ) || matches!(
                            failure,
                            Some(EngineError::Store(StoreError::BudgetExceeded))
                        ),
                        "unrelated failure satisfied raw admission: {:?}, {failure:?}",
                        reads.truncation_reason
                    );
                }
                None => {
                    let failure = reads.take_error();
                    panic!(
                        "measured metadata must return its valid projection: {:?}, {failure:?}",
                        reads.truncation_reason
                    );
                }
            }
            Ok(())
        },
    );
    assert!(
        observed,
        "the real nested store path was never reached: {result:?}"
    );
    assert!(
        result.is_ok(),
        "unexpected authorization/deadline failure: {result:?}"
    );
}

#[test]
fn navigation_does_not_own_large_unneeded_measured_metadata() {
    isolated(
        "tui::input_budget_tests::navigation_does_not_own_large_unneeded_measured_metadata",
        || navigation_cost(true),
    );
}

#[test]
fn navigation_admits_large_legacy_json_before_owned_decoding() {
    isolated(
        "tui::input_budget_tests::navigation_admits_large_legacy_json_before_owned_decoding",
        || navigation_cost(false),
    );
}

#[test]
fn nested_frame_does_not_own_large_unneeded_measured_metadata() {
    isolated(
        "tui::input_budget_tests::nested_frame_does_not_own_large_unneeded_measured_metadata",
        || nested_frame_cost(true),
    );
}

#[test]
fn nested_frame_admits_large_legacy_json_before_owned_decoding() {
    isolated(
        "tui::input_budget_tests::nested_frame_admits_large_legacy_json_before_owned_decoding",
        || nested_frame_cost(false),
    );
}
