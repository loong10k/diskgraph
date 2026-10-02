/// 保存两管道完整输出和正常/信号退出状态；只有执行器成功才可解释。
/// 来源：原生 Rust diskgraph-engine::live_evidence::ProbeOutput。
#[derive(Debug)]
pub(super) struct ProbeOutput {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) exit_code: Option<i32>,
}
