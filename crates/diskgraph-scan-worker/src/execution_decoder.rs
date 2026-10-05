use crate::{
    ExecutionEvent, ExecutionFailure, ExecutionFrame, ExecutionOutcome, Frame, ProtocolLimits,
    WorkerLimits, incremental_frame_decoder::IncrementalFrameDecoder,
    tree_assembler::TreeAssembler,
};
use std::{fmt, io};

/// 父端无阻塞 I/O 的执行 v2 解码器；来源：PF-06，复用原树结构校验和迭代销毁。
/// Hello 声明匹配只检查 DTO 一致性，不证明二进制信任；结果也不代表 OS wait/组空。
pub struct ExecutionDecoder {
    frames: IncrementalFrameDecoder,
    tree: Option<TreeAssembler>,
    failure: Option<ExecutionFailure>,
    expected_target: String,
    expected_pin: String,
    hello: bool,
    terminal: bool,
    delivered: bool,
    failed: Option<io::ErrorKind>,
}

impl ExecutionDecoder {
    /// 参数：limits 为原传输上界，expected_target/pin 为父端受信安装选择的预期声明。
    /// 返回：同一单流账本；非法额度或空/超过 256B 的预期声明在复制前拒绝。
    /// 256B 是受信构建声明的本地准入上界，不是执行文件完整性或授权证明。
    pub fn new(
        limits: ProtocolLimits,
        expected_target: &str,
        expected_pin: &str,
    ) -> io::Result<Self> {
        WorkerLimits::from_protocol(limits)?;
        if expected_target.is_empty()
            || expected_pin.is_empty()
            || expected_target.len() > 256
            || expected_pin.len() > 256
        {
            return Err(invalid("invalid expected worker declaration length"));
        }
        Ok(Self {
            frames: IncrementalFrameDecoder::new(limits),
            tree: Some(TreeAssembler::new(limits)),
            failure: None,
            expected_target: expected_target.to_owned(),
            expected_pin: expected_pin.to_owned(),
            hello: false,
            terminal: false,
            delivered: false,
            failed: None,
        })
    }

    /// 参数：chunk 为同次非阻塞管道读取的借用片段，可含碎片或多帧。
    /// 返回：只消耗到一个完整帧事件；父端须保留余下片段，End/Error 后关闭控制写端。
    /// 失败锁存，不能跳过坏帧续读；方法不等待管道，不改变父端原期限。
    pub fn push(&mut self, chunk: &[u8]) -> io::Result<(usize, Option<ExecutionEvent>)> {
        self.check_failed()?;
        let result = self.push_checked(chunk);
        if let Err(error) = &result {
            self.failed = Some(error.kind());
        }
        result
    }

    fn push_checked(&mut self, chunk: &[u8]) -> io::Result<(usize, Option<ExecutionEvent>)> {
        if self.delivered || (self.terminal && !chunk.is_empty()) {
            return Err(invalid("trailing execution data"));
        }
        let (consumed, body) = self.frames.push(chunk)?;
        let Some(body) = body else {
            return Ok((consumed, None));
        };
        let frame: ExecutionFrame = serde_json::from_slice(&body)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok((consumed, Some(self.accept(frame)?)))
    }

    fn accept(&mut self, frame: ExecutionFrame) -> io::Result<ExecutionEvent> {
        match frame {
            ExecutionFrame::Hello {
                version,
                target,
                pin,
            } => {
                if self.hello
                    || version != 2
                    || target != self.expected_target
                    || pin != self.expected_pin
                {
                    return Err(invalid("unexpected worker Hello"));
                }
                self.hello = true;
                Ok(ExecutionEvent::Hello)
            }
            ExecutionFrame::Error {
                code,
                io_kind,
                raw_os_error,
                message,
            } => {
                let failure = ExecutionFailure::checked(code, io_kind, raw_os_error, message)?;
                if !self.hello && failure.code() != "protocol" {
                    return Err(invalid("failure before worker Hello"));
                }
                drop(self.tree.take());
                self.failure = Some(failure);
                self.terminal = true;
                Ok(ExecutionEvent::Failed)
            }
            ExecutionFrame::Progress { progress } => {
                self.require_hello()?;
                // 进度只观察，不以 finished/cancelled 代替 End、EOF 或退场。
                Ok(ExecutionEvent::Progress(progress))
            }
            ExecutionFrame::Node { node } => {
                self.require_hello()?;
                let tree = self.tree.as_mut().expect("live tree before terminal");
                tree.accept(Frame::Node { node })?;
                Ok(ExecutionEvent::Node {
                    nodes: tree.count(),
                })
            }
            ExecutionFrame::End { nodes } => {
                self.require_hello()?;
                self.tree
                    .as_mut()
                    .expect("live tree before terminal")
                    .accept(Frame::End { nodes })?;
                self.terminal = true;
                Ok(ExecutionEvent::End { nodes })
            }
        }
    }

    fn require_hello(&self) -> io::Result<()> {
        if !self.hello {
            return Err(invalid("result before worker Hello"));
        }
        Ok(())
    }

    /// 参数：self 为原执行解码器。
    /// 返回：已准入的头与声明正文总字节；短正文/JSON 错误不退款，不表示 RSS。
    pub fn bytes_admitted(&self) -> u64 {
        self.frames.bytes()
    }

    /// 参数：self 为底层管道已实际报告 EOF 的原解码器。
    /// 返回：完整终态后的协议结果且仅一次；部分帧、缺终态、尾字节或失败锁存均拒绝。
    /// 父端在获得此结果后仍必须等待真实 OS 退出和自有组/Job 无活动。
    /// 最后组装为 O(nodes) CPU/分配工作，当前不提供 checkpoint 或 20ms 响应保证。
    pub fn finish_eof(&mut self) -> io::Result<ExecutionOutcome> {
        self.check_failed()?;
        let result = self.finish_checked();
        if let Err(error) = &result {
            self.failed = Some(error.kind());
        }
        result
    }

    fn finish_checked(&mut self) -> io::Result<ExecutionOutcome> {
        if self.delivered {
            return Err(invalid("execution result already delivered"));
        }
        self.frames.finish_eof()?;
        if !self.terminal {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "missing execution terminal",
            ));
        }
        self.delivered = true;
        if let Some(failure) = self.failure.take() {
            return Ok(ExecutionOutcome::Failure(failure));
        }
        let tree = self
            .tree
            .take()
            .expect("successful terminal owns validated tree")
            .finish()?;
        Ok(ExecutionOutcome::Tree(tree))
    }

    fn check_failed(&self) -> io::Result<()> {
        if let Some(kind) = self.failed {
            return Err(io::Error::new(kind, "execution decoder already failed"));
        }
        Ok(())
    }
}

impl fmt::Debug for ExecutionDecoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionDecoder")
            .field("bytes_admitted", &self.bytes_admitted())
            .field("hello", &self.hello)
            .field("terminal", &self.terminal)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
