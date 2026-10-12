use std::io::{self, BufRead};

/// 单行stdio请求的有界读取器；来源：原生Rust MCP-02，无Java对等对象。上限与HTTP正文一致为1MiB。
pub(crate) struct BoundedStdioLine {
    bytes: Vec<u8>,
}
impl BoundedStdioLine {
    /// 参数：无；返回：空的有界读取责任，不预分配完整预算。
    pub(crate) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// 参数：原BufRead；返回：完整UTF-8行、真实EOF或超预算/原I/O错误。
    /// 超预算在扩容前拒绝；不消耗或分发后续RPC，避免截断输入被当成完整请求。
    pub(crate) fn read(&mut self, input: &mut impl BufRead) -> io::Result<Option<String>> {
        self.bytes.clear();
        loop {
            let available = match input.fill_buf() {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if available.is_empty() {
                if self.bytes.is_empty() {
                    return Ok(None);
                }
                break;
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(available.len(), |position| position + 1);
            if count > (1 << 20) - self.bytes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stdio request exceeds 1 MiB",
                ));
            }
            // 在上限内几何扩容，避免逐小块精确扩容造成重复复制；容量不会超过固定预算。
            let needed = self.bytes.len() + count;
            if needed > self.bytes.capacity() {
                let capacity = needed.max(self.bytes.capacity() * 2).min(1 << 20);
                self.bytes
                    .try_reserve_exact(capacity - self.bytes.len())
                    .map_err(io::Error::other)?;
            }
            self.bytes.extend_from_slice(&available[..count]);
            input.consume(count);
            if newline.is_some() {
                break;
            }
        }
        if self.bytes.last() == Some(&b'\n') {
            self.bytes.pop();
            if self.bytes.last() == Some(&b'\r') {
                self.bytes.pop();
            }
        }
        String::from_utf8(std::mem::take(&mut self.bytes))
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

#[cfg(test)]
mod tests;
