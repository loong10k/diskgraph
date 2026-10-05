use crate::{DecodedTree, ExecutionDecodeError};
use diskgraph_disktree_core::tree::Node;
use std::io;

const BATCH: usize = 256;

/// 部分组装森林的独占 owner；来源：PF-06，失败与 unwind 都采用预留空间迭代销毁。
pub(crate) struct TreeAssembly {
    nodes: Vec<(Option<u64>, Node)>,
    result: Option<DecodedTree>,
}

impl TreeAssembly {
    /// 参数：nodes 为已验证且尚无子树的平铺节点。
    /// 返回：拥有全部节点及预留清理空间的森林；分配失败保持原错误。
    pub(crate) fn new(nodes: Vec<(Option<u64>, Node)>) -> io::Result<Self> {
        let result = DecodedTree::prepare(nodes.len())?;
        Ok(Self {
            nodes,
            result: Some(result),
        })
    }

    /// 参数：checkpoint 借用原请求，不刷新期限或克隆停止原因。
    /// 返回：完整有序树或原错误；任何失败均不泄露部分组装结果。
    pub(crate) fn finish<E>(
        mut self,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<DecodedTree, ExecutionDecodeError<E>> {
        checkpoint().map_err(ExecutionDecodeError::Checkpoint)?;
        let mut counts = Vec::new();
        counts
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| io::Error::other("child count allocation failed"))?;
        while counts.len() < self.nodes.len() {
            checkpoint().map_err(ExecutionDecodeError::Checkpoint)?;
            counts.resize((counts.len() + BATCH).min(self.nodes.len()), 0_usize);
        }
        let mut operations = 0;
        for (parent, _) in &self.nodes {
            check_step(&mut operations, &mut checkpoint)?;
            if let Some(parent) = parent {
                let parent = usize::try_from(*parent)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "parent overflow"))?;
                counts[parent] = counts[parent].checked_add(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "child count overflow")
                })?;
            }
        }
        for ((_, node), count) in self.nodes.iter_mut().zip(counts) {
            check_step(&mut operations, &mut checkpoint)?;
            node.children
                .try_reserve_exact(count)
                .map_err(|_| io::Error::other("children allocation failed"))?;
        }
        while self.nodes.len() > 1 {
            // 检查反转时仍让森林持有 child；失败或 panic 不能把深子树交给递归 Drop。
            let (_, child) = self.nodes.last_mut().expect("validated nonempty");
            reverse_checked(&mut child.children, &mut operations, &mut checkpoint)?;
            check_step(&mut operations, &mut checkpoint)?;
            let (parent, child) = self.nodes.pop().expect("validated nonempty");
            self.nodes[parent.expect("validated parent") as usize]
                .1
                .children
                .push(child);
        }
        reverse_checked(
            &mut self.nodes[0].1.children,
            &mut operations,
            &mut checkpoint,
        )?;
        check_step(&mut operations, &mut checkpoint)?;
        let (_, root) = self.nodes.pop().expect("validated root");
        self.result
            .as_mut()
            .expect("forest disposal owner")
            .install(root);
        checkpoint().map_err(ExecutionDecodeError::Checkpoint)?;
        Ok(self.result.take().expect("completed tree owner"))
    }
}

impl Drop for TreeAssembly {
    fn drop(&mut self) {
        if let Some(result) = self.result.as_mut() {
            while let Some((_, node)) = self.nodes.pop() {
                result.discard(node);
            }
        }
    }
}

fn reverse_checked<E>(
    children: &mut [Node],
    operations: &mut usize,
    checkpoint: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), ExecutionDecodeError<E>> {
    for index in 0..children.len() / 2 {
        check_step(operations, checkpoint)?;
        children.swap(index, children.len() - index - 1);
    }
    Ok(())
}

// 全部遍历共享余额；不能让宽分支反转与后续合并各自重新获得256次额度。
fn check_step<E>(
    operations: &mut usize,
    checkpoint: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), ExecutionDecodeError<E>> {
    if *operations == 0 {
        checkpoint().map_err(ExecutionDecodeError::Checkpoint)?;
    }
    #[cfg(test)]
    crate::assembly_operation_tests::operation();
    *operations = (*operations + 1) % BATCH;
    Ok(())
}
