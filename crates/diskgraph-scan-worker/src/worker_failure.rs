use crate::worker_io_kind::WorkerIoKind;
use crate::worker_message::WorkerMessage;
use serde::{Serialize, Serializer, ser::SerializeStruct};
use std::io;

/// 专用 v2 失败帧；来源：真实 std::io::Error，旧 Frame::Error 不增加必填构造字段。
pub(crate) struct WorkerFailure {
    frame_type: &'static str,
    code: &'static str,
    io_kind: WorkerIoKind,
    raw_os_error: Option<i32>,
    error: io::Error,
}

impl WorkerFailure {
    /// 参数：code 为运行阶段的固定代码，error 为真实 IO/协议错误。
    /// 返回：保留原错误类型和 OS 码的失败，不提前格式化可能很长的 message。
    pub(crate) fn new(code: &'static str, error: io::Error) -> Self {
        Self {
            frame_type: "error",
            code,
            io_kind: WorkerIoKind::from_native(error.kind()),
            raw_os_error: error.raw_os_error(),
            error,
        }
    }
}

impl Serialize for WorkerFailure {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_struct("WorkerFailure", 5)?;
        fields.serialize_field("type", self.frame_type)?;
        fields.serialize_field("code", self.code)?;
        fields.serialize_field("io_kind", &self.io_kind)?;
        fields.serialize_field("raw_os_error", &self.raw_os_error)?;
        fields.serialize_field("message", &WorkerMessage(&self.error))?;
        fields.end()
    }
}
