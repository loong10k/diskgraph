use crate::FlatNode;
use diskgraph_disktree_core::tree::Node;
use std::{io, slice};

/// 保留上游子节点顺序的借用先序迭代器；来源：pinned Node，无递归 serde 或子树复制。
pub struct FlatNodes<'a> {
    root: Option<&'a Node>,
    stack: Vec<(slice::Iter<'a, Node>, u64, u64)>,
    next_sequence: u64,
    failed: bool,
}

impl<'a> FlatNodes<'a> {
    /// 开始遍历。参数 root 为真实扫描树；返回 O(depth) 遍历状态。
    /// 参数：root 为调用者保持存活的原生扫描树。
    /// 返回：尚未拥有任何名称、按原子节点顺序迭代的遍历器。
    pub fn new(root: &'a Node) -> Self {
        Self {
            root: Some(root),
            stack: Vec::new(),
            next_sequence: 0,
            failed: false,
        }
    }

    /// 在复制名称之前准入真实原生节点。
    /// 参数：limits 为原节点/深度额度，max_name_bytes 为原帧和剩余流正文容量的最小值。
    /// 返回：下一平铺记录或错误；名称原字节仅是 JSON 必要下界，完整转义成本由写 sink 再检查。
    pub(crate) fn next_with_limits(
        &mut self,
        limits: crate::ProtocolLimits,
        max_name_bytes: u64,
    ) -> Option<io::Result<FlatNode>> {
        if self.failed {
            return None;
        }
        let result = self.next_checked(limits, max_name_bytes);
        self.failed = result.as_ref().is_some_and(Result::is_err);
        result
    }

    fn next_checked(
        &mut self,
        limits: crate::ProtocolLimits,
        max_name_bytes: u64,
    ) -> Option<io::Result<FlatNode>> {
        let (node, parent, depth) = if let Some(root) = self.root.take() {
            (root, None, 0)
        } else {
            loop {
                let (children, parent, depth) = self.stack.last_mut()?;
                if let Some(child) = children.next() {
                    break (child, Some(*parent), *depth);
                }
                self.stack.pop();
            }
        };
        let sequence = self.next_sequence;
        let Some(next) = sequence.checked_add(1) else {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sequence overflow",
            )));
        };
        if next > limits.max_nodes
            || depth > limits.max_depth
            || node.name.len() as u64 > max_name_bytes
        {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "node preparation limit",
            )));
        }
        self.next_sequence = next;
        if !node.children.is_empty() {
            let Some(child_depth) = depth.checked_add(1) else {
                return Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "depth overflow",
                )));
            };
            if self.stack.try_reserve(1).is_err() {
                return Some(Err(io::Error::other("traversal stack allocation failed")));
            }
            self.stack
                .push((node.children.iter(), sequence, child_depth));
        }
        Some(Ok(FlatNode::from_native(node, sequence, parent, depth)))
    }
}

impl Iterator for FlatNodes<'_> {
    type Item = io::Result<FlatNode>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_with_limits(
            crate::ProtocolLimits {
                max_frame_bytes: u64::MAX,
                max_stream_bytes: u64::MAX,
                max_nodes: u64::MAX,
                max_depth: u64::MAX,
            },
            u64::MAX,
        )
    }
}
