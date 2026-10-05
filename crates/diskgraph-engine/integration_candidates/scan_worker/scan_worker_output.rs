use crate::scan_worker_failure::ScanWorkerFailure;
use diskgraph_scan_worker::{
    ExecutionDecodeError, ExecutionDecoder, ExecutionEvent, ExecutionOutcome, ProtocolLimits,
    ScanProgress,
};
use std::io;

/// stdout 协议与 stderr 子限额共享原总额，先按头部准入再交解码器拥有正文。
/// 来源：PF-06；头部游标只做双管道成本准入，拓扑和真实 Error 仍由 ExecutionDecoder 判断。
pub(super) struct ScanWorkerOutput {
    decoder: ExecutionDecoder,
    limits: ProtocolLimits,
    stderr_limit: u64,
    stderr_bytes: u64,
    stdout_reserved: u64,
    header: [u8; 4],
    header_used: usize,
    body_remaining: usize,
    terminal: bool,
    outcome: Option<ExecutionOutcome>,
    assembled: bool,
    progress: Option<ScanProgress>,
}

impl ScanWorkerOutput {
    /// 参数：limits 为同一响应上界，expected 为预期声明，stderr_limit 为其内部子限额。
    /// 返回：未消费输出的原账本；声明只作匹配，不作为可信文件或 OS 权限证明。
    pub(super) fn new(
        limits: ProtocolLimits,
        expected: (&str, &str),
        stderr_limit: u64,
    ) -> io::Result<Self> {
        if stderr_limit > limits.max_stream_bytes {
            return Err(invalid("stderr sublimit exceeds response total"));
        }
        Ok(Self {
            decoder: ExecutionDecoder::new(limits, expected.0, expected.1)?,
            limits,
            stderr_limit,
            stderr_bytes: 0,
            stdout_reserved: 0,
            header: [0; 4],
            header_used: 0,
            body_remaining: 0,
            terminal: false,
            outcome: None,
            assembled: false,
            progress: None,
        })
    }

    /// 参数：bytes 为 Child 固定缓冲区的借用片段，check 借用原请求。
    /// 返回：全片段消费或原失败；完整帧声明在正文复制／serde 分配前扣共享账本。
    pub(super) fn stdout<E>(
        &mut self,
        mut bytes: &[u8],
        check: &mut impl FnMut() -> Result<(), ScanWorkerFailure<E>>,
    ) -> Result<(), ScanWorkerFailure<E>> {
        while !bytes.is_empty() {
            check()?;
            if self.terminal {
                return Err(invalid("trailing execution data").into());
            }
            let count = if self.body_remaining == 0 {
                let count = (4 - self.header_used).min(bytes.len());
                self.shared_total(self.stdout_reserved, self.header_used + count)?;
                self.header[self.header_used..self.header_used + count]
                    .copy_from_slice(&bytes[..count]);
                self.header_used += count;
                if self.header_used == 4 {
                    let body = u64::from(u32::from_le_bytes(self.header));
                    if body > self.limits.max_frame_bytes {
                        return Err(invalid("frame byte limit").into());
                    }
                    let reserved = self
                        .stdout_reserved
                        .checked_add(4)
                        .and_then(|n| n.checked_add(body))
                        .ok_or(ScanWorkerFailure::OutputLimit { cleanup: None })?;
                    self.shared_total(reserved, 0)?;
                    self.body_remaining =
                        usize::try_from(body).map_err(|_| invalid("frame length overflow"))?;
                    self.stdout_reserved = reserved;
                    self.header_used = 0;
                }
                count
            } else {
                let count = self.body_remaining.min(bytes.len());
                self.body_remaining -= count;
                count
            };
            let (consumed, event) = self.decoder.push(&bytes[..count])?;
            if consumed != count {
                return Err(invalid("execution decode cursor mismatch").into());
            }
            match event {
                Some(ExecutionEvent::Progress(progress)) => self.progress = Some(progress),
                Some(ExecutionEvent::End { .. } | ExecutionEvent::Failed) => self.terminal = true,
                _ => {}
            }
            bytes = &bytes[count..];
        }
        check()
    }

    /// 参数：count 为真实 Child stderr 读取长度，不保存或打印原生正文。
    /// 返回：原子式双管道准入，已声明 stdout 正文不因暂未到齐而退还。
    pub(super) fn stderr<E>(&mut self, count: usize) -> Result<(), ScanWorkerFailure<E>> {
        let bytes = self
            .stderr_bytes
            .checked_add(count as u64)
            .filter(|n| *n <= self.stderr_limit)
            .ok_or(ScanWorkerFailure::OutputLimit { cleanup: None })?;
        let total = self
            .stdout_reserved
            .checked_add(self.header_used as u64)
            .and_then(|n| n.checked_add(bytes))
            .filter(|n| *n <= self.limits.max_stream_bytes)
            .ok_or(ScanWorkerFailure::OutputLimit { cleanup: None })?;
        let _ = total;
        self.stderr_bytes = bytes;
        Ok(())
    }

    fn shared_total<E>(
        &self,
        stdout: u64,
        partial_header: usize,
    ) -> Result<(), ScanWorkerFailure<E>> {
        stdout
            .checked_add(partial_header as u64)
            .and_then(|n| n.checked_add(self.stderr_bytes))
            .filter(|n| *n <= self.limits.max_stream_bytes)
            .ok_or(ScanWorkerFailure::OutputLimit { cleanup: None })?;
        Ok(())
    }

    /// 参数：无；返回：已解码 End/Error，仅用于关闭输入，仍不是 EOF 或退出许可。
    pub(super) fn terminal(&self) -> bool {
        self.terminal
    }

    /// 参数：check 借同一原绝对期限与权限／fence 检查；调用方已真实观察 stdout EOF。
    /// 返回：完整协议组装事实，最多每256节点／边原检查；原 E 不变，结果暂存不交付。
    pub(super) fn finish<E>(
        &mut self,
        check: &mut impl FnMut() -> Result<(), ScanWorkerFailure<E>>,
    ) -> Result<(), ScanWorkerFailure<E>> {
        if !self.assembled {
            let result =
                self.decoder
                    .finish_eof_with_checkpoint(check)
                    .map_err(|error| match error {
                        ExecutionDecodeError::Protocol(error) => ScanWorkerFailure::from(error),
                        ExecutionDecodeError::Checkpoint(error) => error,
                    })?;
            self.outcome = Some(result);
            self.assembled = true;
        }
        Ok(())
    }

    /// 参数：无；返回：实际已组装的完整协议结果是否为成功树，不能推断 leader 退出码。
    pub(super) fn tree(&self) -> bool {
        matches!(self.outcome, Some(ExecutionOutcome::Tree(_)))
    }

    /// 参数：无；返回：暂存完整结果并仅转移一次；调用者先证明 Child 正常退出许可。
    pub(super) fn take(&mut self) -> Option<ExecutionOutcome> {
        self.outcome.take()
    }

    /// 参数：无；返回：最后一个真实 Progress 全字段借用，finished 不等于物理退出。
    pub(super) fn progress(&self) -> Option<&ScanProgress> {
        self.progress.as_ref()
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
