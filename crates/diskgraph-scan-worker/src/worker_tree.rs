use diskgraph_disktree_core::tree::Node;

/// helper 私有原生树 owner；来源：pinned Node，迭代销毁避免深目录递归 Drop。
/// 清理工作区无法分配时终止本 helper，由 OS 回收；不返回伪成功或退回递归释放。
pub(crate) struct WorkerTree {
    root: Option<Node>,
}

impl WorkerTree {
    /// 参数：root 为 pinned scanner 交付的真实唯一树。
    /// 返回：负责非递归清理的私有 owner，不克隆节点或名称。
    pub(crate) fn new(root: Node) -> Self {
        Self { root: Some(root) }
    }

    /// 参数：self 为仍持有树的 owner。
    /// 返回：原树借用，不能把递归所有权移出清理边界。
    pub(crate) fn root(&self) -> &Node {
        self.root.as_ref().expect("worker tree remains installed")
    }
}

impl Drop for WorkerTree {
    fn drop(&mut self) {
        let Some(root) = self.root.take() else {
            return;
        };
        let mut pending = Vec::new();
        if pending.try_reserve(1).is_err() {
            std::process::abort();
        }
        pending.push(root);
        while let Some(mut node) = pending.pop() {
            if pending.try_reserve(node.children.len()).is_err() {
                std::process::abort();
            }
            pending.append(&mut node.children);
        }
    }
}
