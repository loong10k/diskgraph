use crate::{
    DecodedTree, ExecutionDecodeError, Frame, ProtocolLimits, tree_assembly::TreeAssembly,
    tree_state::TreeState,
};
use diskgraph_disktree_core::tree::Node;
use std::io;

/// 阻塞 codec 与执行 v2 共用的真实树组装器；来源：原 read_tree，失败阶段只持无子树 Node。
pub(crate) struct TreeAssembler {
    state: TreeState,
    nodes: Vec<(Option<u64>, Node)>,
}

impl TreeAssembler {
    /// 参数：limits 为同次平铺树的原节点及深度上界。
    /// 返回：保留原 TreeState 判定规则的空组装器。
    pub(crate) fn new(limits: ProtocolLimits) -> Self {
        Self {
            state: TreeState::new(limits),
            nodes: Vec::new(),
        }
    }

    /// 参数：frame 为下一完整结果帧，不得传入 Hello 或输入命令。
    /// 返回：沿原结构校验后接管实际无子树 Node；不按声明 child_count 预分配。
    pub(crate) fn accept(&mut self, frame: Frame) -> io::Result<()> {
        self.state.accept(&frame)?;
        if let Frame::Node { node } = frame {
            let parent = node.parent;
            let mut native = node.into_native()?;
            native.children.clear();
            self.nodes
                .try_reserve(1)
                .map_err(|_| io::Error::other("node allocation failed"))?;
            self.nodes.push((parent, native));
        }
        Ok(())
    }

    /// 参数：self 为同次树校验状态。
    /// 返回：已经接收的节点数，不含进度或声明值。
    pub(crate) fn count(&self) -> u64 {
        self.state.count()
    }

    /// 参数：self 为同次树校验状态。
    /// 返回：完整 End 是否已通过原校验；不证明 EOF 或 OS 退场。
    pub(crate) fn ended(&self) -> bool {
        self.state.ended()
    }

    /// 参数：self 为完整 End 和外部 EOF 均已验证的组装器。
    /// 返回：预留迭代销毁空间的真实树；分配完成前不形成递归子树所有权。
    pub(crate) fn finish(self) -> io::Result<DecodedTree> {
        self.finish_with_checkpoint(|| Ok::<(), io::Error>(()))
            .map_err(|error| match error {
                ExecutionDecodeError::Protocol(error) | ExecutionDecodeError::Checkpoint(error) => {
                    error
                }
            })
    }

    /// 参数：checkpoint 是原请求的实时检查，不创建新额度。
    /// 返回：完整树或保留原对象的停止错误；部分深树由森林 owner 迭代销毁。
    pub(crate) fn finish_with_checkpoint<E>(
        self,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<DecodedTree, ExecutionDecodeError<E>> {
        checkpoint().map_err(ExecutionDecodeError::Checkpoint)?;
        if !self.state.ended() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing End").into());
        }
        TreeAssembly::new(self.nodes)?.finish(checkpoint)
    }
}
