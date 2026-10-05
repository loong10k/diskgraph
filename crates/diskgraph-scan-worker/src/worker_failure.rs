use crate::{
    ExecutionFrame, ProtocolBudgetError, worker_io_kind::WorkerIoKind,
    worker_message::WorkerMessage,
};
use serde::{Serialize, Serializer};
use std::io;

/// 专用 v2 失败帧；来源：真实 std::io::Error，旧 Frame::Error 不增加必填构造字段。
pub struct WorkerFailure {
    code: &'static str,
    io_kind: WorkerIoKind,
    raw_os_error: Option<i32>,
    error: io::Error,
}

impl WorkerFailure {
    /// 参数：code 为运行阶段的固定代码，error 为真实 IO/协议错误。
    /// 返回：保留原错误类型和 OS 码的失败，不提前格式化可能很长的 message。
    pub fn new(code: &'static str, error: io::Error) -> Self {
        // 仅实际输出额度类型升级固定代码；其他阶段、IO 类别及消息不作为预算见证。
        let code = if code == "output"
            && error
                .get_ref()
                .is_some_and(|source| source.is::<ProtocolBudgetError>())
        {
            "output_budget"
        } else {
            code
        };
        Self {
            code,
            io_kind: WorkerIoKind::from_native(error.kind()),
            raw_os_error: error.raw_os_error(),
            error,
        }
    }
}

impl Serialize for WorkerFailure {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ExecutionFrame::Error {
            code: self.code,
            io_kind: self.io_kind,
            raw_os_error: self.raw_os_error,
            message: WorkerMessage(&self.error),
        }
        .serialize(serializer)
    }
}
