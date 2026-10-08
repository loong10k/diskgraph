//! 原协议树销毁不得新增分配；来源：PF-06原owner与非递归释放合同。
use diskgraph_disktree_core::tree::Node;
use diskgraph_scan_worker::{FlatNode, Frame, ProtocolLimits, read_tree, write_frame};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// 当前测试线程的实际分配见证；来源：std::alloc::GlobalAlloc透明System委托。
struct AllocationWitness;

fn record_allocation() {
    if TRACK.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get().saturating_add(1)));
    }
}

unsafe impl GlobalAlloc for AllocationWitness {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: AllocationWitness = AllocationWitness;

fn check_disposal(count: u64, deep: bool) {
    let mut bytes = Vec::new();
    for sequence in 0..count {
        let name = format!("node-{sequence}");
        let mut node = FlatNode::from_native(
            &Node::directory(name.as_str()),
            sequence,
            if deep {
                sequence.checked_sub(1)
            } else {
                (sequence > 0).then_some(0)
            },
            if deep {
                sequence
            } else {
                u64::from(sequence > 0)
            },
        );
        node.child_count = if deep {
            u64::from(sequence + 1 < count)
        } else if sequence == 0 {
            count - 1
        } else {
            0
        };
        write_frame(&mut bytes, &Frame::Node { node }).unwrap();
    }
    write_frame(&mut bytes, &Frame::End { nodes: count }).unwrap();
    let tree = read_tree(
        bytes.as_slice(),
        ProtocolLimits {
            max_frame_bytes: 4096,
            max_stream_bytes: 32 * 1024 * 1024,
            max_nodes: count,
            max_depth: count,
        },
    )
    .unwrap();
    assert_eq!(
        tree.children.len(),
        if deep { 1 } else { (count - 1) as usize }
    );
    ALLOCATIONS.with(|value| value.set(0));
    TRACK.with(|value| value.set(true));
    drop(tree);
    TRACK.with(|value| value.set(false));
    assert_eq!(
        ALLOCATIONS.with(Cell::get),
        0,
        "original tree Drop allocated"
    );
}

#[test]
fn wide_original_tree_drop_does_not_allocate() {
    check_disposal(20_001, false);
}

#[test]
fn deep_original_tree_drop_on_small_stack_does_not_allocate() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| check_disposal(10_000, true))
        .unwrap()
        .join()
        .unwrap();
}
