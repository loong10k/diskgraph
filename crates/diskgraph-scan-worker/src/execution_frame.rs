use crate::{FlatNode, Frame, ScanProgress, WorkerIoKind};
use serde::{Deserialize, Serialize};
use std::io;

/// 唯一执行 v2 输出 DTO；来源：真实 WorkerOutput/WorkerFailure，不改变旧 Frame 的兼容形状。
/// C/M 允许真实错误代码及延迟格式化消息直接借用序列化；接收端使用默认 String 闭合解码。
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionFrame<C = String, M = String> {
    Hello {
        version: u32,
        target: String,
        pin: String,
    },
    Progress {
        progress: ScanProgress,
    },
    Node {
        node: FlatNode,
    },
    End {
        nodes: u64,
    },
    Error {
        code: C,
        io_kind: WorkerIoKind,
        raw_os_error: Option<i32>,
        message: M,
    },
}

impl ExecutionFrame {
    /// 参数：frame 为 helper 已通过原 TreeState 校验的旧结果帧。
    /// 返回：同一字段与顺序的 v2 输出；旧 Request/Cancel/缺少真实 IO 类别的 Error 拒绝。
    pub(crate) fn from_result_frame(frame: Frame) -> io::Result<Self> {
        match frame {
            Frame::Hello {
                version,
                target,
                pin,
            } => Ok(Self::Hello {
                version,
                target,
                pin,
            }),
            Frame::Progress { progress } => Ok(Self::Progress { progress }),
            Frame::Node { node } => Ok(Self::Node { node }),
            Frame::End { nodes } => Ok(Self::End { nodes }),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an execution result frame",
            )),
        }
    }
}
