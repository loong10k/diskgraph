use crate::{
    ExecutionFrame, FrameWriter, ProtocolBudgetError, ProtocolLimits,
    prepared_payload::PreparedPayload, worker_failure::WorkerFailure,
};
use std::io::{self, Write};

/// 执行请求与真实输出共用的 Hello/预算终态准入；来源：PF-06 原累计传输额度合同。
/// 按实际有限编码确定空间，不放大额度、不写出准备结果、不改变通用解码器语义。
pub(crate) struct WorkerTerminalBudget;

impl WorkerTerminalBudget {
    /// 参数：limits 为已经通过结构校验的原四项额度。
    /// 返回：真实 Hello 与最大固定预算终态均可完整承载，或原准备错误；不启动扫描。
    pub(crate) fn check(limits: ProtocolLimits) -> io::Result<()> {
        let mut frames = FrameWriter::new(io::sink(), limits);
        Self::prepare(&mut frames).map(|_| ())
    }

    /// 参数：frames 为同次原输出账本；返回：只序列化一次的 Hello 与完整终态预留字节。
    /// 六种固定错误走真实 WorkerFailure 编码；准备失败仍保留 writer 原错误分类及锁存规则。
    pub(crate) fn prepare<W: Write>(
        frames: &mut FrameWriter<W>,
    ) -> io::Result<(PreparedPayload, u64)> {
        let mut reserve = 0;
        for message in [
            "frame byte limit",
            "stream byte limit",
            "node preparation limit",
            "node count limit",
            "sequence or depth limit",
            "declared children exceed node limit",
        ] {
            let error = WorkerFailure::new("output", ProtocolBudgetError::into_io(message));
            let prepared = frames.prepare_reserving(&error, 0)?;
            reserve = reserve.max(prepared.as_slice().len() as u64 + 4);
        }
        let hello = frames.prepare_reserving(
            &ExecutionFrame::<String, String>::Hello {
                version: 2,
                target: env!("DISKGRAPH_WORKER_TARGET").to_owned(),
                pin: "158f9cc2f0b332194a3ffc5acec47760c99146d8".to_owned(),
            },
            reserve,
        )?;
        Ok((hello, reserve))
    }
}
