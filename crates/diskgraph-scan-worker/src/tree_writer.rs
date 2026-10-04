use crate::{Frame, FrameWriter, ProtocolLimits, tree_state::TreeState};
use std::io::{self, Write};

/// 结构与字节账本共用的结果写端；来源：PF-06，不能把控制或握手帧混入结果阶段。
pub struct TreeWriter<W> {
    frames: FrameWriter<W>,
    state: TreeState,
    failed: bool,
}

impl<W: Write> TreeWriter<W> {
    /// 绑定同一次流及原上界。返回未消费的结果写端，不建立任何 OS 进程。
    /// 参数：output 为结果流，limits 为其原节点/深度/字节上界。
    /// 返回：共用这些上界的结构状态与写入账本。
    pub fn new(output: W, limits: ProtocolLimits) -> Self {
        Self {
            frames: FrameWriter::new(output, limits),
            state: TreeState::new(limits),
            failed: false,
        }
    }

    /// 写入真实 Progress/Node/End。参数 frame 必须符合当前阶段；错误锁存且不发出非法帧。
    /// 参数：frame 为下一条 Progress、Node 或 End。
    /// 返回：合法帧完整发出的成功或锁存错误。
    pub fn write_frame(&mut self, frame: &Frame) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("tree writer already failed"));
        }
        let result = self
            .state
            .accept(frame)
            .and_then(|()| self.frames.write_frame(frame).map(|_| ()));
        self.failed = result.is_err();
        result
    }

    /// 检查 End 已发出。返回真实节点数；只确认协议结束，不代替 pipe EOF 或 OS wait。
    /// 参数：self 为待结束的结果写端。
    /// 返回：成功 End 的真实节点数；失败或缺 End 不得返回成功。
    pub fn finish(self) -> io::Result<u64> {
        if self.failed || !self.state.ended() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "missing successful End",
            ));
        }
        Ok(self.state.count())
    }

    /// 在建立拥有名称的平铺记录之前借用当前正文余额。
    /// 参数：self 为同一次结果流；返回：原 frame/stream 剩余额度或锁存失败。
    pub(crate) fn remaining_body_bytes(&self) -> io::Result<u64> {
        if self.failed {
            return Err(io::Error::other("tree writer already failed"));
        }
        self.frames.remaining_body_bytes()
    }

    /// 返回完整写出的传输字节，保留头部成本且不重置配额。
    /// 参数：self 为当前结果写端。
    /// 返回：完整发出的头部与正文总字节，不刷新额度。
    pub fn bytes_written(&self) -> u64 {
        self.frames.bytes_written()
    }
}
