use std::io;

/// 解码终态的协议错误或原请求停止原因；来源：PF-06 原生 Rust 父端检查点合同。
/// 原原因不要求 Clone/Display，也不转换为通用协议失败。
#[derive(Debug)]
pub enum ExecutionDecodeError<E> {
    /// 原始协议、分配或已锁存错误。
    Protocol(io::Error),
    /// 原请求检查函数返回的真实停止对象。
    Checkpoint(E),
}

impl<E> From<io::Error> for ExecutionDecodeError<E> {
    fn from(error: io::Error) -> Self {
        Self::Protocol(error)
    }
}
