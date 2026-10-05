use crate::{DecodedTree, ExecutionFailure};

/// End/Error 后干净 EOF 才可领取一次的协议结果；来源：PF-06，仍需父 Child owner 的 OS 退出证明。
#[derive(Debug)]
pub enum ExecutionOutcome {
    Tree(DecodedTree),
    Failure(ExecutionFailure),
}
