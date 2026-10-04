//! 可信 Linux 观察兼容入口；来源：Rust D42，产品执行器持有相同租约直到编码后复核。
use super::ProcessNativeSession;
use super::linux_observation_lease::LinuxObservationLease;
use diskgraph_core::{
    IndexedFileEpoch, ProcessEvidenceFailureCode as Failure, ProcessEvidenceSummary,
};
use std::path::Path;

impl ProcessNativeSession<'_> {
    /// 参数：实际授权根、已索引相对普通文件与扫描强 epoch；返回：逐资源启动身份与保守覆盖。
    /// 调用者负责实际 owner/scope；同会话累计预算与原认领时钟不重置，不读取目标正文。
    pub fn observe_linux_file(
        &self,
        root: &Path,
        relative: &Path,
        expected: &IndexedFileEpoch,
    ) -> Result<ProcessEvidenceSummary, Failure> {
        LinuxObservationLease::open(root, relative, expected, self)?.observe()
    }
}
