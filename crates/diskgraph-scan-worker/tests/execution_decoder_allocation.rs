//! 仅巨大 4B 头部的真实 Rust 分配准入；不计 RSS、SQLite、OS I/O 或物理退出。
use diskgraph_scan_worker::{ExecutionDecoder, ProtocolLimits};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static MEASURING: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);

/// 独占测试子进程的透明计量器；来源：Rust System，布局与分配结果原样转交。
struct MeasuredAllocator;
// SAFETY: 委托每一指针与 Layout 给 System；仅用无分配原子记录成功请求量。
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

fn isolated(name: &str, work: impl FnOnce()) {
    const CHILD: &str = "DISKGRAPH_EXECUTION_HEADER_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(name) {
        work();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("test result: ok. 1 passed"));
}

fn refuse_header(bound: ProtocolLimits) {
    let mut input = ExecutionDecoder::new(bound, "target", "pin").unwrap();
    let header = (2 * 1024 * 1024_u32).to_le_bytes();
    REQUESTED.store(0, Ordering::Relaxed);
    MEASURING.store(true, Ordering::Relaxed);
    let result = input.push(&header);
    MEASURING.store(false, Ordering::Relaxed);
    let requested = REQUESTED.load(Ordering::Relaxed);
    println!("EXECUTION_HEADER requested={requested} result={result:?}");
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert!(
        requested < 65_536,
        "2MiB body was reserved before header admission: {requested}"
    );
    assert_eq!(input.bytes_admitted(), 0);
    assert!(input.push(&[]).is_err());
    assert!(input.finish_eof().is_err());
}

#[test]
fn oversized_single_frame_is_refused_before_body_allocation() {
    isolated(
        "oversized_single_frame_is_refused_before_body_allocation",
        || {
            refuse_header(ProtocolLimits {
                max_frame_bytes: 1024,
                max_stream_bytes: 4 * 1024 * 1024,
                max_nodes: 2,
                max_depth: 1,
            });
        },
    );
}

#[test]
fn oversized_stream_charge_is_refused_before_body_allocation() {
    isolated(
        "oversized_stream_charge_is_refused_before_body_allocation",
        || {
            refuse_header(ProtocolLimits {
                max_frame_bytes: 4 * 1024 * 1024,
                max_stream_bytes: 1024,
                max_nodes: 2,
                max_depth: 1,
            });
        },
    );
}
