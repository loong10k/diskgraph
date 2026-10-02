//! 只在调用方内存中携带读取字节及范围/稳定性诊断，不持久化正文。

use super::InspectionStop;
use std::path::PathBuf;

/// 只在调用方内存中携带读取字节及范围/稳定性诊断，不持久化正文。
/// 来源：原生 Rust diskgraph-engine::content::ReadOutcome。
/// What a bounded read produced. The bytes are the caller's to use and drop;
/// nothing else in this structure can carry content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadOutcome {
    pub requested_path: PathBuf,
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub file_len: u64,
    pub truncated: bool,
    /// Set when the read did not complete for a named reason; `bytes` then
    /// holds whatever was read before the stop and must not be trusted as
    /// the file's content.
    pub stopped: Option<InspectionStop>,
    pub observed_at_unix_ms: u64,
}

impl ReadOutcome {
    /// 生成不携带正文的元数据日志。
    /// 参数：无；使用当前检查结果。
    /// 返回：文件名、范围、长度、截断/中止及时间描述，不输出正文或摘要。
    /// A log line for this read: names, ranges, flags — never bytes (CT-04).
    pub fn redacted_log(&self) -> String {
        format!(
            "read {} bytes[{}..{}] len={} truncated={} stopped={:?} at={}",
            self.requested_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<unnamed>".into()),
            self.offset,
            self.offset + self.bytes.len() as u64,
            self.file_len,
            self.truncated,
            self.stopped,
            self.observed_at_unix_ms
        )
    }
}
