use diskgraph_scan_worker::ExecutionFailure;
use std::{fmt, io};

/// 已资格helper发出的有界I/O事实，类别/原码/消息独立保留，不从消息推断授权。
/// 来源：原生 Rust 执行v2 ExecutionFailure；不是父端原生io::Error对象。
#[derive(Debug)]
pub(super) struct ScanWorkerRemoteError {
    failure: ExecutionFailure,
    native: Option<io::Error>,
}

impl ScanWorkerRemoteError {
    /// 参数：failure为闭合解码的原Error帧；返回：保持原类别并可沿source查看原码的I/O包装。
    pub(super) fn into_io(failure: ExecutionFailure) -> io::Error {
        let kind = failure.io_kind();
        let native = failure.raw_os_error().map(io::Error::from_raw_os_error);
        io::Error::new(kind, Self { failure, native })
    }
}

impl fmt::Display for ScanWorkerRemoteError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            output,
            "scan worker {}: {}",
            self.failure.code(),
            self.failure.message()
        )
    }
}
impl std::error::Error for ScanWorkerRemoteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.native
            .as_ref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}
