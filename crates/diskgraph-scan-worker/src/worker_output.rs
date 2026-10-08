use crate::{
    ExecutionFrame, FlatNodes, Frame, FrameWriter, ProtocolLimits, ScanProgress,
    tree_state::TreeState, worker_control::WorkerControl, worker_failure::WorkerFailure,
    worker_terminal_budget::WorkerTerminalBudget,
};
use diskgraph_disktree_core::{scan::ScanSnapshot, tree::Node};
use std::io::{self, Write};

/// 执行期唯一输出账本；来源：PF-06，握手、真实进度、平铺树和错误都不重置额度。
pub(crate) struct WorkerOutput<W> {
    frames: FrameWriter<W>,
    state: TreeState,
    limits: ProtocolLimits,
    terminal_reserve: u64,
}

impl<W: Write> WorkerOutput<W> {
    /// 参数：output 为 stdout 或测试管道，limits 为已校验的原执行请求额度。
    /// 返回：整个执行期共用的单一输出 owner。
    pub(crate) fn new(output: W, limits: ProtocolLimits) -> Self {
        Self {
            frames: FrameWriter::new(output, limits),
            state: TreeState::new(limits),
            limits,
            terminal_reserve: 0,
        }
    }

    /// 为实际管道构造固定 64 KiB 缓冲。
    /// 参数：output 为原管道 writer，limits 为已校验的原协议额度。
    /// 返回：同一协议账本；Hello、Progress、End 和 Error 显式刷新，不能借 Drop 证明交付。
    pub(crate) fn for_pipe(output: W, limits: ProtocolLimits) -> WorkerOutput<io::BufWriter<W>> {
        WorkerOutput::new(io::BufWriter::with_capacity(64 << 10, output), limits)
    }

    /// 参数：self 为未发送结果的输出端。
    /// 返回：真实构建 target 与固定 pin 声明的写入结果；声明不证明安装完整性。
    pub(crate) fn hello(&mut self) -> io::Result<()> {
        let (hello, reserve) = WorkerTerminalBudget::prepare(&mut self.frames)?;
        self.terminal_reserve = reserve;
        self.frames.write_prepared(hello)?;
        self.frames.flush()
    }

    /// 参数：snapshot 为 pinned scanner 的真实进度快照，所有字段及 messages 保留。
    /// 返回：共用原额度的 Progress 写入结果。
    pub(crate) fn progress(&mut self, snapshot: ScanSnapshot) -> io::Result<()> {
        let frame = Frame::Progress {
            progress: ScanProgress::from_native(snapshot),
        };
        self.state.accept(&frame)?;
        self.data(&ExecutionFrame::from_result_frame(frame)?)?;
        self.frames.flush()
    }

    /// 参数：root 为唯一原生树的借用，control 为同次实际取消信号。
    /// 返回：完整 End 成功或真实失败；每个名称拥有前检查原节点/深度/正文剩余额度。
    pub(crate) fn tree(
        &mut self,
        root: &Node,
        control: &WorkerControl,
    ) -> Result<(), WorkerFailure> {
        let mut nodes = FlatNodes::new(root);
        loop {
            control.check()?;
            let remaining = self.frames.remaining_data_bytes(self.terminal_reserve);
            // 没有下一节点时 End 可消费原终态余额，无需为不存在的数据帧再扣头部。
            let Some(node) =
                nodes.next_with_limits(self.limits, remaining.as_ref().copied().unwrap_or(0))
            else {
                break;
            };
            remaining.map_err(output_error)?;
            let frame = Frame::Node {
                node: node.map_err(output_error)?,
            };
            self.state.accept(&frame).map_err(output_error)?;
            let frame = ExecutionFrame::from_result_frame(frame).map_err(output_error)?;
            self.data(&frame).map_err(output_error)?;
        }
        control.check()?;
        let end = Frame::End {
            nodes: self.state.count(),
        };
        self.state.accept(&end).map_err(output_error)?;
        // 先标记合法终帧，父收到 End 后的关闭不会与控制 reader 产生假 UnexpectedEof。
        control.mark_terminal();
        let end = ExecutionFrame::from_result_frame(end).map_err(output_error)?;
        self.frames.write_payload(&end).map_err(output_error)?;
        self.frames.flush().map_err(output_error)
    }

    /// 参数：failure 为固定类别及真实 IO 消息，self 仍使用原流余额。
    /// 返回：唯一 Error 帧写入结果；部分 IO 失败后不重建 writer 续写。
    pub(crate) fn failure(&mut self, failure: &WorkerFailure) -> io::Result<()> {
        self.frames.write_payload(failure)?;
        self.frames.flush()
    }

    fn data<T: serde::Serialize + ?Sized>(&mut self, payload: &T) -> io::Result<()> {
        let prepared = self
            .frames
            .prepare_reserving(payload, self.terminal_reserve)?;
        self.frames.write_prepared(prepared)?;
        Ok(())
    }
}

fn output_error(error: io::Error) -> WorkerFailure {
    WorkerFailure::new("output", error)
}
