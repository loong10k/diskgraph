//! 隔离进程计量解析分配，验证重复句柄不会放大命令存储；来源：D27 / EV-06。
use super::UsageCoverage;
use super::process_output::interpret_process_output;
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static MEASURING: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

/// 测试分配器仅在隔离用例的测量窗口累计请求字节，不改变生产分配器。
/// 来源：原生 Rust D27 分配回归夹具。
struct MeasuredAllocator;

// SAFETY: 原样转发布局/指针/尺寸给 System，仅通过无分配原子操作记录成功请求。
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            ALLOCATED.fetch_add(size, Ordering::Relaxed);
        }
        pointer
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

#[test]
fn repeated_handles_do_not_multiply_command_allocations() {
    const CHILD: &str = "DISKGRAPH_PROCESS_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // 独占测试进程消除并行测试分配干扰；父进程不启用计量。
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "live_evidence::process_allocation_tests::repeated_handles_do_not_multiply_command_allocations",
                "--exact", "--nocapture", "--test-threads=1",
            ])
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
    let command = "c".repeat(16_384);
    let mut output = format!("p42\0c{command}\0").into_bytes();
    for _ in 0..4096 {
        output.extend_from_slice(b"nfixture\0");
    }
    assert!(output.len() < 1 << 20);
    ALLOCATED.store(0, Ordering::Relaxed);
    MEASURING.store(true, Ordering::Relaxed);
    let sample = interpret_process_output(&output, Some(0), &[Path::new("fixture")], 1);
    MEASURING.store(false, Ordering::Relaxed);
    let allocated = ALLOCATED.load(Ordering::Relaxed);
    println!(
        "input_bytes={}, cumulative_allocated_bytes={allocated}",
        output.len()
    );
    assert_eq!(sample.holders.len(), 1);
    assert_eq!(sample.holders[0].command, command);
    assert!(matches!(sample.coverage, UsageCoverage::Partial { .. }));
    assert!(
        allocated < 2 << 20,
        "bounded input expanded to {allocated} allocated bytes"
    );
}
