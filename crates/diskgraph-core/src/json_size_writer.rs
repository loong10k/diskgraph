use std::io::{self, Write};

/// 仅计数的有界 JSON writer，不保留序列化字节。
/// 来源：DiskGraph 原生 Rust 查询响应计量；无 Java 对应对象。
pub(crate) struct JsonSizeWriter {
    size: usize,
    limit: usize,
    exceeded: bool,
}

/// 按真实 Serialize 编码计量而不生成完整 Vec。
/// 参数：value 为数据或实际 envelope；limit 是当前独立编码字节上限。
/// 返回：Some 为精确尺寸，None 为超限/溢出；真实 Serialize 错误保留。
pub fn measure_json_bounded<T: serde::Serialize + ?Sized>(
    value: &T,
    limit: usize,
) -> Result<Option<usize>, serde_json::Error> {
    let mut writer = JsonSizeWriter {
        size: 0,
        limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(Some(writer.size)),
        Err(_) if writer.exceeded => Ok(None),
        Err(error) => Err(error),
    }
}

impl Write for JsonSizeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(size) = self
            .size
            .checked_add(bytes.len())
            .filter(|size| *size <= self.limit)
        else {
            self.exceeded = true;
            return Err(io::Error::other("query JSON byte budget exceeded"));
        };
        self.size = size;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
