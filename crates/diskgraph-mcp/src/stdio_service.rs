//! 逐行读取 JSON-RPC，stdout 仅输出协议帧，诊断写入独立日志目标。来源：原生 Rust MCP 服务。
use crate::McpService;
use crate::protocol::{FrameError, decode_request, log_line, protocol_error};
use serde_json::Value;
use std::io::{BufRead, Write};

/// 逐行读取 JSON-RPC，stdout 仅输出协议帧，诊断写入独立日志目标。
/// 参数：service 为共享服务，input/output 为帧输入输出，log 为诊断目标。返回：完成结果或原 I/O 错误。
pub fn serve_stdio<R: BufRead, W: Write, L: Write>(
    service: &mut McpService,
    input: R,
    output: &mut W,
    mut log: L,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match decode_request(&line) {
            Ok(request) => {
                let response = service.handle(&request);
                writeln!(
                    output,
                    "{}",
                    serde_json::to_string(&response).unwrap_or_default()
                )?;
                output.flush()?;
            }
            Err(FrameError::Batch) => {
                // Batched frames are refused explicitly rather than partially
                // executed; the id is null because the frame could not be read.
                writeln!(
                    output,
                    "{}",
                    serde_json::to_string(&protocol_error(
                        Value::Null,
                        -32600,
                        "batched requests are not supported; send one frame per line",
                    ))
                    .unwrap_or_default()
                )?;
                output.flush()?;
            }
            Err(error) => {
                let reason = match error {
                    FrameError::Malformed => "malformed",
                    FrameError::MissingMethod => "missing_method",
                    FrameError::Batch => "batch",
                };
                writeln!(log, "{}", log_line("frame_rejected", &[("reason", reason)]))?;
            }
        }
    }
    Ok(())
}
