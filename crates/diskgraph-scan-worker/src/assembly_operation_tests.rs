use crate::{FlatNode, Frame, ProtocolLimits, tree_assembler::TreeAssembler};
use diskgraph_disktree_core::tree::Node;
use std::cell::RefCell;

thread_local! {
    static LEDGER: RefCell<Option<(usize, usize)>> = const { RefCell::new(None) };
}

pub(crate) fn operation() {
    LEDGER.with(|ledger| {
        if let Some((gap, maximum)) = ledger.borrow_mut().as_mut() {
            *gap += 1;
            *maximum = (*maximum).max(*gap);
        }
    });
}

#[test]
fn reversal_and_merge_share_one_original_checkpoint_operation_budget() {
    // 真正结构反控：255次分支反转后还有两个合并；两套局部计数会出现257次间隔。
    let limits = ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 1_048_576,
        max_nodes: 513,
        max_depth: 2,
    };
    let mut tree = TreeAssembler::new(limits);
    for sequence in 0..513 {
        let (parent, depth, children) = match sequence {
            0 => (None, 0, 2),
            1 => (Some(0), 1, 0),
            2 => (Some(0), 1, 510),
            _ => (Some(2), 2, 0),
        };
        let mut node = FlatNode::from_native(&Node::directory("observed"), sequence, parent, depth);
        node.child_count = children;
        tree.accept(Frame::Node { node }).unwrap();
    }
    tree.accept(Frame::End { nodes: 513 }).unwrap();
    LEDGER.with(|ledger| *ledger.borrow_mut() = Some((0, 0)));
    let decoded = tree
        .finish_with_checkpoint(|| {
            LEDGER.with(|ledger| ledger.borrow_mut().as_mut().unwrap().0 = 0);
            Ok::<(), ()>(())
        })
        .unwrap();
    let (_, maximum) = LEDGER.with(|ledger| ledger.borrow_mut().take().unwrap());
    assert!(maximum <= 256, "actual observed operation gap: {maximum}");
    assert_eq!(decoded.children[1].children.len(), 510);
}
