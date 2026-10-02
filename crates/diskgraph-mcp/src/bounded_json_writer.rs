use std::io::{self, Write};

/// 有界 JSON 编码缓冲；超限立即停止写入，不先生成完整的大字符串。
/// 来源：DiskGraph 原生 Rust HTTP/legacy SSE 编码；无 Java 对应对象。
pub(crate) struct BoundedJsonWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedJsonWriter {
    /// 编码响应。参数：value 为结果，limit 为字节上限；返回：完整 JSON 或超限 None。
    pub(crate) fn encode(value: &serde_json::Value, limit: usize) -> Option<String> {
        let mut writer = Self {
            bytes: Vec::with_capacity(limit.min(1024)),
            limit,
        };
        serde_json::to_writer(&mut writer, value).ok()?;
        String::from_utf8(writer.bytes).ok()
    }
}

impl Write for BoundedJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("response byte budget exceeded"));
        }
        // 按预算封顶的倍增保持摊销线性，转义密集字符串也不逐字节重新分配。
        let needed = self.bytes.len() + buffer.len();
        if needed > self.bytes.capacity() {
            let capacity = needed
                .max(self.bytes.capacity().saturating_mul(2))
                .min(self.limit);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::BoundedJsonWriter;

    #[test]
    fn unicode_and_escaped_results_are_complete_or_refused_by_encoded_bytes() {
        let value = serde_json::json!({"path":"\\\n中文🙂".repeat(256)});
        let expected = value.to_string();
        assert_eq!(
            BoundedJsonWriter::encode(&value, expected.len()),
            Some(expected.clone())
        );
        assert_eq!(BoundedJsonWriter::encode(&value, expected.len() - 1), None);
        assert_eq!(BoundedJsonWriter::encode(&value, 0), None);
    }

    #[test]
    fn oversized_write_is_refused_before_allocating_or_retaining_its_bytes() {
        let mut writer = BoundedJsonWriter {
            bytes: Vec::new(),
            limit: 128,
        };
        assert!(writer.write_all(&[0; 1024]).is_err());
        assert!(writer.bytes.is_empty());
        assert_eq!(writer.bytes.capacity(), 0);
        for _ in 0..128 {
            writer.write_all(b"x").unwrap();
        }
        assert_eq!(writer.bytes.len(), 128);
        assert!(writer.bytes.capacity() <= 128);
        assert!(writer.write_all(b"x").is_err());
    }
}
