use diskgraph_disktree_core::tree::Node;
use std::{fmt, io, ops::Deref};

/// 已完成结构校验的树与迭代销毁工作区；来源：PF-06，防止深 Node 递归 Drop 溢出。
/// 只代表协议树，绝不代表 OS 退出许可；不提供逃逸为裸 Node 的消费接口。
pub struct DecodedTree {
    root: Option<Node>,
    pending: Vec<std::vec::IntoIter<Node>>,
}

impl DecodedTree {
    /// 参数：count 为已验证平铺节点总数。
    /// 返回：预留足够迭代器帧容量的空树 owner，分配失败返回错误。
    pub(crate) fn prepare(count: usize) -> io::Result<Self> {
        let mut pending = Vec::new();
        pending
            .try_reserve_exact(count)
            .map_err(|_| io::Error::other("tree disposal allocation failed"))?;
        Ok(Self {
            root: None,
            pending,
        })
    }
    /// 参数：root 为已验证并组装完成的唯一根节点。
    /// 返回：无返回值；将根交给已有 owner 负责非递归销毁。
    pub(crate) fn install(&mut self, root: Node) {
        self.root = Some(root);
    }

    /// 参数：node 为同次组装森林的一棵子树，不超过 prepare 的原节点数。
    /// 返回：无返回值；使用已有工作区迭代销毁，不分配或递归下降。
    pub(crate) fn discard(&mut self, node: Node) {
        let mut current = Some(node);
        loop {
            if let Some(mut node) = current.take() {
                // 迭代器接管原children缓冲；不复制宽目录，也不让Node递归销毁子树。
                let children = std::mem::take(&mut node.children);
                if !children.is_empty() {
                    self.pending.push(children.into_iter());
                }
            }
            let Some(frame) = self.pending.last_mut() else {
                break;
            };
            if let Some(node) = frame.next() {
                current = Some(node);
            } else {
                // 仅弹出已经耗尽的迭代器；其Drop不会再递归处理未访问的Node。
                self.pending.pop();
            }
        }
    }
}

impl Deref for DecodedTree {
    type Target = Node;
    fn deref(&self) -> &Node {
        self.root
            .as_ref()
            .expect("validated root installed before return")
    }
}

impl fmt::Debug for DecodedTree {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedTree")
            .field("present", &self.root.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for DecodedTree {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            // 最多count个祖先帧已预留；每条边访问一次，Node销毁时children为空。
            self.discard(root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DecodedTree;

    #[test]
    fn disposal_workspace_does_not_reserve_a_second_full_node_array() {
        let count = 20_000;
        let mut owner = DecodedTree::prepare(count).unwrap();
        // 测量实际预留存储，限制为每个已验证节点最多八个机器字；不是RSS上限。
        let reserved = std::mem::size_of_val(owner.pending.spare_capacity_mut());
        let limit = count * 8 * std::mem::size_of::<usize>();
        eprintln!("disposal workspace nodes={count} reserved_bytes={reserved} limit_bytes={limit}");
        assert!(
            reserved <= limit,
            "disposal reservation {reserved} exceeds {limit}"
        );
    }
}
