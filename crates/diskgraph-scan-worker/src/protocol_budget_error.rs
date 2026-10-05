use std::{fmt, io};

/// 本地协议校验实际耗尽调用者额度的类型见证，不由消息或远端 IO 类别推断。
/// 来源：原生 Rust PF-06 / OpenSpec 15.2；保留既有 InvalidData 与原诊断文本。
#[derive(Debug)]
pub struct ProtocolBudgetError {
    message: &'static str,
}

impl ProtocolBudgetError {
    /// 参数：message 为实际额度检查点的固定原诊断。
    /// 返回：保留 InvalidData 类别并携带可 downcast 类型的错误；构造仅限协议 crate。
    pub(crate) fn into_io(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, Self { message })
    }
}

impl fmt::Display for ProtocolBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for ProtocolBudgetError {}
