//! 独占子进程中的 Rust 成功 requested 字节测量；不代表 SQLite C 分配、磁盘读取或 RSS。
use crate::StoreError;
use crate::git_job_test_fixtures::{batch, fixture};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 仅测试启用的同进程分配观测器；来源：原生 Rust 实际 Store 公共读入口回归。
struct ObservedAllocator;
static ENABLED: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOCATOR: ObservedAllocator = ObservedAllocator;

unsafe impl GlobalAlloc for ObservedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && ENABLED.load(Ordering::Relaxed) {
            REQUESTED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && ENABLED.load(Ordering::Relaxed) {
            REQUESTED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, new_size) };
        if !result.is_null() && ENABLED.load(Ordering::Relaxed) {
            REQUESTED.fetch_add(new_size, Ordering::Relaxed);
        }
        result
    }
}
fn measure<T>(work: impl FnOnce() -> T) -> (T, usize) {
    REQUESTED.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    let result = work();
    ENABLED.store(false, Ordering::SeqCst);
    (result, REQUESTED.load(Ordering::Relaxed))
}
fn isolated(name: &str) -> bool {
    if std::env::var("DISKGRAPH_GIT_RAW_ALLOCATION_CHILD").as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env("DISKGRAPH_GIT_RAW_ALLOCATION_CHILD", name)
        .output()
        .unwrap();
    eprintln!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "isolated requested-allocation observation failed"
    );
    true
}

#[test]
fn input_redundant_server_column_is_checked_before_owned_allocation() {
    let name = "git_raw_allocation_tests::input_redundant_server_column_is_checked_before_owned_allocation";
    if isolated(name) {
        return;
    }
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let (positive, bytes) = measure(|| control.git_evidence_job_input(&job.job_id));
    assert_eq!(positive.unwrap(), input);
    assert!(bytes < 65536, "ordinary input requested {bytes}");
    control
        .connection
        .execute("UPDATE server SET server_id=?1", ["x".repeat(2 << 20)])
        .unwrap();
    let (result, bytes) = measure(|| control.git_evidence_job_input(&job.job_id));
    assert!(matches!(result, Err(StoreError::InvalidGraph(_))));
    eprintln!("input redundant 2MiB server field; Rust requested={bytes}");
    assert!(
        bytes < 65536,
        "an invalid redundant field was owned before validation: {bytes}"
    );
}

#[test]
fn receipt_redundant_server_column_is_checked_before_owned_allocation() {
    let name = "git_raw_allocation_tests::receipt_redundant_server_column_is_checked_before_owned_allocation";
    if isolated(name) {
        return;
    }
    let (_, mut graph, input, _) = fixture();
    let receipt = graph
        .publish_git_collector_revision_checked(
            "job-one",
            &input,
            (1, 1001),
            "published",
            &batch(&input, "run-one"),
            || Ok(()),
        )
        .unwrap();
    let (positive, bytes) = measure(|| graph.job_publication_receipt("job-one"));
    assert_eq!(positive.unwrap(), Some(receipt));
    assert!(bytes < 65536, "ordinary receipt requested {bytes}");
    graph
        .connection
        .execute_batch("DROP TRIGGER job_receipt_no_update;")
        .unwrap();
    graph
        .connection
        .execute(
            "UPDATE job_publication_receipts SET server_id=?1",
            ["x".repeat(2 << 20)],
        )
        .unwrap();
    let (result, bytes) = measure(|| graph.job_publication_receipt("job-one"));
    assert!(matches!(result, Err(StoreError::InvalidGraph(_))));
    eprintln!("receipt redundant 2MiB server field; Rust requested={bytes}");
    assert!(
        bytes < 65536,
        "an invalid redundant field was owned before validation: {bytes}"
    );
}
