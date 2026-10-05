use crate::native_child::ControlWriteStatus;
use crate::scan_worker_input_buffer::ScanWorkerInputBuffer;
use diskgraph_scan_worker::{FrameWriter, ProtocolLimits, WorkerInputLimits, WorkerRequest};
use std::io;

/// 原 Request 与至多一次 Cancel 的顺序发送游标，仅拥有一个待发帧。
/// 来源：PF-06 WorkerControl；Pending 只轮询 Child 已拥有的同一数据块。
pub(super) struct ScanWorkerInput {
    buffer: ScanWorkerInputBuffer,
    limits: ProtocolLimits,
    admitted: u64,
    position: usize,
    pending: Option<usize>,
    request_done: bool,
    cancel_requested: bool,
    cancel_frame: bool,
    cancel_done: bool,
    closing: bool,
    close_polling: bool,
    closed: bool,
}

impl ScanWorkerInput {
    /// 参数：request 为借用的唯一执行请求；输入额度取 helper 的固定合同。
    /// 返回：原 FrameWriter 已有限序列化的 Request，不克隆无界原始 DTO。
    pub(super) fn new(request: &WorkerRequest) -> io::Result<Self> {
        let limits = WorkerInputLimits::protocol_limits();
        let mut buffer = ScanWorkerInputBuffer::new(limits.max_stream_bytes);
        let admitted = FrameWriter::new(&mut buffer, limits).write_payload(request)?;
        Ok(Self {
            buffer,
            limits,
            admitted,
            position: 0,
            pending: None,
            request_done: false,
            cancel_requested: false,
            cancel_frame: false,
            cancel_done: false,
            closing: false,
            close_polling: false,
            closed: false,
        })
    }

    /// 参数：requested 是本次调用观察的真实 caller 取消标志。
    /// 返回：取消被锁存，之后先完整 Request 再发送一次 Cancel；终帧关闭优先。
    pub(super) fn request_cancel(&mut self, requested: bool) {
        self.cancel_requested |= requested;
    }

    /// 参数：无；返回：caller 是否真实请求过取消，不从 helper 进度或文本推断。
    pub(super) fn cancelled(&self) -> bool {
        self.cancel_requested
    }

    /// 参数：self 为原帧游标，Child 尚未持有待完成块。
    /// 返回：最多4096字节借用块，或当前无帧；Cancel 仅消费原剩余输入额度。
    pub(super) fn chunk(&mut self) -> io::Result<Option<&[u8]>> {
        if self.closing || self.pending.is_some() {
            return Ok(None);
        }
        if self.position == self.buffer.bytes().len() {
            if self.cancel_frame {
                self.cancel_done = true;
            } else {
                self.request_done = true;
            }
            if !self.request_done || !self.cancel_requested || self.cancel_done {
                return Ok(None);
            }
            let remaining = self
                .limits
                .max_stream_bytes
                .checked_sub(self.admitted)
                .ok_or_else(|| invalid("input stream byte limit"))?;
            self.buffer.reset(remaining);
            let limits = ProtocolLimits {
                max_stream_bytes: remaining,
                ..self.limits
            };
            let written = FrameWriter::new(&mut self.buffer, limits)
                .write_payload(&WorkerRequest::Cancel {})?;
            self.admitted = self
                .admitted
                .checked_add(written)
                .ok_or_else(|| invalid("input stream size overflow"))?;
            self.position = 0;
            self.cancel_frame = true;
        }
        let end = self
            .position
            .saturating_add(ControlWriteStatus::MAX_CHUNK_BYTES)
            .min(self.buffer.bytes().len());
        Ok(Some(&self.buffer.bytes()[self.position..end]))
    }

    /// 参数：self 为本次输入状态。
    /// 返回：Child 是否正持有未完成块，true 时禁止重新开始写入。
    pub(super) fn pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 参数：status 来自真实 Child 操作，started 为本次新块长度或既有 pending。
    /// 返回：只按实际正数推进；零完成、超出块和未请求关闭均明确拒绝。
    pub(super) fn observe(&mut self, status: ControlWriteStatus, started: usize) -> io::Result<()> {
        let length = self.pending.unwrap_or(started);
        match status {
            ControlWriteStatus::Written(count) if count > 0 && count <= length => {
                self.position = self
                    .position
                    .checked_add(count)
                    .filter(|position| *position <= self.buffer.bytes().len())
                    .ok_or_else(|| invalid("control write cursor overflow"))?;
                self.pending = None;
                Ok(())
            }
            ControlWriteStatus::Pending if length > 0 => {
                self.pending = Some(length);
                Ok(())
            }
            _ => Err(invalid("unexpected control write completion")),
        }
    }

    /// 参数：无；返回：原终帧后的关闭状态，不再插入新 Request 或 Cancel。
    pub(super) fn begin_close(&mut self) {
        self.closing = true;
    }

    /// 参数：无；返回：是否已看到终帧而必须封闭输入。
    pub(super) fn closing(&self) -> bool {
        self.closing
    }

    /// 参数：无；返回：是否应只收取既有 CancelIoEx 完成，不能再发新写操作。
    pub(super) fn close_polling(&self) -> bool {
        self.close_polling
    }

    /// 参数：status 为真实关闭或完成轮询结果。
    /// 返回：Pending 不释放原生 buffer；Written 竞争只记一次，后续仍要求 Closed。
    pub(super) fn observe_close(&mut self, status: ControlWriteStatus) -> io::Result<()> {
        match status {
            ControlWriteStatus::Closed => {
                self.closed = true;
                self.close_polling = false;
                self.pending = None;
            }
            ControlWriteStatus::Pending => self.close_polling = true,
            ControlWriteStatus::Written(count) if count > 0 => {
                self.close_polling = false;
                self.pending = None;
            }
            ControlWriteStatus::Written(_) => return Err(invalid("zero control close completion")),
        }
        Ok(())
    }

    /// 参数：无；返回：实际原生控制端已 Closed，不以 Cancel 已排队代替关闭。
    pub(super) fn closed(&self) -> bool {
        self.closed
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
