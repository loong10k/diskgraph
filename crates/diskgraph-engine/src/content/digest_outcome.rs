//! 保存实际读取成本与摘要状态；中止时摘要为空且不能确认。

use super::InspectionStop;
use std::path::PathBuf;

/// 保存实际读取成本与摘要状态；中止时摘要为空且不能确认。
/// 来源：原生 Rust diskgraph-engine::content::DigestOutcome。
/// A content digest with the stability evidence that makes it meaningful.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DigestOutcome {
    pub requested_path: PathBuf,
    pub digest_hex: String,
    pub bytes_digested: u64,
    pub stopped: Option<InspectionStop>,
    pub observed_at_unix_ms: u64,
}

impl DigestOutcome {
    /// 检查摘要是否具有未中止的确认状态。
    /// 参数：无。
    /// 返回：未中止为 true；调用方仍需检查其授权上下文。
    pub fn confirmed(&self) -> bool {
        self.stopped.is_none()
    }

    /// 生成不携带正文的元数据日志。
    /// 参数：无；使用当前检查结果。
    /// 返回：文件名、范围/摘要及状态描述，不输出正文。
    /// A log line for this digest: the file name and the digest, no content.
    pub fn redacted_log(&self) -> String {
        format!(
            "digest {} {} confirmed={} at={}",
            self.requested_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<unnamed>".into()),
            self.digest_hex,
            self.confirmed(),
            self.observed_at_unix_ms
        )
    }
}
