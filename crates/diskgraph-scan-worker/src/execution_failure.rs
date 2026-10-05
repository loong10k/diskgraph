use crate::WorkerIoKind;
use std::io;

/// 已闭合解码的真实 helper 失败事实；来源：WorkerFailure，不授予授权或发布许可。
#[derive(Debug)]
pub struct ExecutionFailure {
    code: String,
    io_kind: WorkerIoKind,
    raw_os_error: Option<i32>,
    message: String,
}

impl ExecutionFailure {
    /// 参数：各字段来自已准入且闭合解码的 Error 帧。
    /// 返回：固定阶段集合内的失败；未知阶段及预算代码的非法类别/OS 码组合拒绝。
    /// 消息不用于推断错误类型，Hello 顺序继续由 ExecutionDecoder 验证。
    pub(crate) fn checked(
        code: String,
        io_kind: WorkerIoKind,
        raw_os_error: Option<i32>,
        message: String,
    ) -> io::Result<Self> {
        if !matches!(
            code.as_str(),
            "protocol" | "control" | "scan_io" | "output" | "cancelled" | "output_budget"
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown execution failure phase",
            ));
        }
        if code == "output_budget"
            && (!matches!(io_kind, WorkerIoKind::InvalidData) || raw_os_error.is_some())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid execution budget failure facts",
            ));
        }
        Ok(Self {
            code,
            io_kind,
            raw_os_error,
            message,
        })
    }

    /// 参数：self 为本次 helper 的失败记录。
    /// 返回：固定运行阶段代码，不是业务授权结果。
    pub fn code(&self) -> &str {
        &self.code
    }

    /// 参数：self 为本次 helper 的失败记录。
    /// 返回：原生产者记录的标准库类别；未知原生扩展仍为 Other。
    pub fn io_kind(&self) -> io::ErrorKind {
        self.io_kind.into_native()
    }

    /// 参数：self 为本次 helper 的失败记录。
    /// 返回：真实 OS 错误码或 None，保留独立于类别和文本的事实。
    pub fn raw_os_error(&self) -> Option<i32> {
        self.raw_os_error
    }

    /// 参数：self 为本次 helper 的失败记录。
    /// 返回：有界帧内的原消息；父端仍须按产品契约控制展示，不能解析成权限或成功。
    pub fn message(&self) -> &str {
        &self.message
    }
}
