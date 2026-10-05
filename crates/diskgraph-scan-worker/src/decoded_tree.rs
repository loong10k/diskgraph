use diskgraph_disktree_core::tree::Node;
use std::{fmt, io, ops::Deref};

/// 已完成结构校验的树与迭代销毁工作区；来源：PF-06，防止深 Node 递归 Drop 溢出。
/// 只代表协议树，绝不代表 OS 退出许可；不提供逃逸为裸 Node 的消费接口。
pub struct DecodedTree {
    root: Option<Node>,
    pending: Vec<Node>,
}

impl DecodedTree {
    /// 参数：count 为已验证平铺节点总数。
    /// 返回：预留足够迭代清理容量的空树 owner，分配失败返回错误。
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
        self.pending.push(node);
        while let Some(mut node) = self.pending.pop() {
            self.pending.append(&mut node.children);
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
            // count 已预留，每条边移动一次，Node 自身销毁时 children 为空。
            self.discard(root);
        }
    }
}
