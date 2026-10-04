//! PF-06 bounded tree writer 的整个公开调用准备成本；来源：原生 Rust System 分配计量。
//! 输入大名称在窗口外构造；只计 Rust requested bytes，不等同 RSS、OS I/O 或峰值存活内存。
use diskgraph_disktree_core::{
    classify::{Category, Reclaim},
    tree::{Node, NodeKind},
};
use diskgraph_scan_worker::{
    FlatNode, FlatNodes, Frame, ProtocolLimits, read_tree, write_frame, write_tree_with_limits,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static MEASURING: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
const LARGE_NAME: usize = 2 * 1024 * 1024;
const REFUSAL_CEILING: usize = 512 * 1024;

/// 独占子进程的透明 System 计量器；不改变分配成功/失败或布局语义。
struct MeasuredAllocator;
// SAFETY: 所有原始指针、Layout 和 size 原样委托给 System，计数原子不触发分配。
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

fn measured<T>(work: impl FnOnce() -> T) -> (T, usize) {
    REQUESTED.store(0, Ordering::Relaxed);
    MEASURING.store(true, Ordering::Relaxed);
    let result = work();
    MEASURING.store(false, Ordering::Relaxed);
    (result, REQUESTED.load(Ordering::Relaxed))
}

fn isolated(name: &str, work: impl FnOnce()) {
    const CHILD: &str = "DISKGRAPH_WRITER_PREPARATION_CHILD";
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

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 4 * 1024 * 1024,
        max_stream_bytes: 16 * 1024 * 1024,
        max_nodes: 8,
        max_depth: 4,
    }
}

fn refuse_before_name_copy(root: &Node, bound: ProtocolLimits, expected_output: &[u8]) {
    let mut output = Vec::new();
    let (result, requested) = measured(|| write_tree_with_limits(&mut output, root, bound));
    println!(
        "writer_preparation requested={requested} result={result:?} output_bytes={}",
        output.len()
    );
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        output.as_slice(),
        expected_output,
        "no offending frame header or body may be emitted"
    );
    assert!(
        requested < REFUSAL_CEILING,
        "bounded writer copied a 2MiB owned name before refusal: {requested}"
    );
}

#[test]
fn zero_node_quota_refuses_before_cloning_large_owned_root_name() {
    isolated(
        "zero_node_quota_refuses_before_cloning_large_owned_root_name",
        || {
            let root = Node::directory("r".repeat(LARGE_NAME));
            let mut bound = limits();
            bound.max_nodes = 0;
            refuse_before_name_copy(&root, bound, &[]);
        },
    );
}

#[test]
fn tiny_frame_refuses_before_cloning_large_owned_root_name() {
    isolated(
        "tiny_frame_refuses_before_cloning_large_owned_root_name",
        || {
            let root = Node::directory("r".repeat(LARGE_NAME));
            let mut bound = limits();
            bound.max_frame_bytes = 32;
            refuse_before_name_copy(&root, bound, &[]);
        },
    );
}

#[test]
fn exhausted_stream_remainder_refuses_before_cloning_next_child_name() {
    isolated(
        "exhausted_stream_remainder_refuses_before_cloning_next_child_name",
        || {
            let mut root = Node::directory("/root");
            root.children
                .push(Node::entry("f".repeat(LARGE_NAME), NodeKind::File, 19));
            let mut prefix = Vec::new();
            write_frame(
                &mut prefix,
                &Frame::Node {
                    node: FlatNode::from_native(&root, 0, None, 0),
                },
            )
            .unwrap();
            let mut bound = limits();
            bound.max_stream_bytes = prefix.len() as u64 + 4 + 32;
            refuse_before_name_copy(&root, bound, &prefix);
        },
    );
}

#[test]
fn ample_budget_preserves_large_name_and_all_node_fields() {
    isolated(
        "ample_budget_preserves_large_name_and_all_node_fields",
        || {
            let mut root = Node::directory("/root");
            let mut child = Node::entry("f".repeat(LARGE_NAME), NodeKind::File, 1234);
            child.own_bytes = 456;
            child.files = 2;
            child.own_files = 3;
            child.dirs = 4;
            child.inode = Some((u64::MAX, 918));
            child.modified = -7;
            child.read_error = true;
            child.category = Category::AgentScratch;
            child.reclaim = Some(Reclaim::Temporary);
            root.children.push(child);
            root.children
                .push(Node::entry("second", NodeKind::Symlink, 9));
            let mut bytes = Vec::new();
            let (result, requested) =
                measured(|| write_tree_with_limits(&mut bytes, &root, limits()));
            println!(
                "writer_preparation ample requested={requested} result={result:?} output_bytes={}",
                bytes.len()
            );
            assert_eq!(result.unwrap(), 3);
            let decoded = read_tree(bytes.as_slice(), limits()).unwrap();
            let before: Vec<_> = FlatNodes::new(&root).collect::<Result<_, _>>().unwrap();
            let after: Vec<_> = FlatNodes::new(&decoded).collect::<Result<_, _>>().unwrap();
            assert_eq!(before, after);
            assert_eq!(decoded.children[0].name.len(), LARGE_NAME);
            assert_eq!(decoded.children[1].name.as_ref(), "second");
        },
    );
}
