use super::ChildError;
use std::io;

/// 出生前准备的固定逻辑长度读缓冲，移动进程 owner 时只转移唯一分配。
/// 来源：原生 Rust PF-06 父端管道与显式回收合同，无 Java 对等对象。
pub(super) struct ChildReadBuffer {
    bytes: Vec<u8>,
}

impl ChildReadBuffer {
    /// 参数：无；返回：逻辑长度4096的唯一缓冲，或原分配失败，不创建 child。
    /// 容量由分配器决定；不声称精确 RSS，也不把分配失败改写成协议额度错误。
    pub(super) fn new() -> Result<Self, ChildError> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(4096).map_err(|error| {
            ChildError::io("prepare child read buffer", io::Error::other(error))
        })?;
        // 已预留完整长度，初始化不会再次申请内存；之后不扩容或暴露 Vec。
        bytes.resize(4096, 0);
        Ok(Self { bytes })
    }

    /// 参数：无；返回：借用原固定长度字节，不复制、不转移底层分配。
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 参数：无；返回：供一次原生读使用的固定长度借用，不提供扩容能力。
    pub(super) fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}
