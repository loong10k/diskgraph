use diskgraph_core::{DiskNode, EvidenceEdge, TruncationReason};

/// 有界候选、已选字节、缺口和完整性诊断，不代表批准。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// 一次候选审阅的结果；字节缺口和截断状态不能被当成删除授权。
#[derive(Clone, Debug, serde::Serialize)]
pub struct CandidateSelection {
    pub candidates: Vec<(DiskNode, Vec<EvidenceEdge>)>,
    pub selected_bytes: u64,
    pub remaining_bytes: u64,
    pub coverage_complete: bool,
    pub complete: bool,
    pub truncated: Option<TruncationReason>,
}

impl CandidateSelection {
    /// 准备有界审阅候选、目标缺口和真实截断状态。
    /// 参数：target_bytes：审阅目标字节数；coverage_complete：观测覆盖是否完整。
    /// 返回：空候选、目标缺口及输入覆盖状态。
    pub(crate) fn empty(target_bytes: u64, coverage_complete: bool) -> Self {
        Self {
            candidates: Vec::new(),
            selected_bytes: 0,
            remaining_bytes: target_bytes,
            coverage_complete,
            complete: coverage_complete,
            truncated: None,
        }
    }

    /// 准备有界审阅候选、目标缺口和真实截断状态。
    /// 参数：reason：预算截断原因。
    /// 返回：无返回值；候选变为不完整并记录真实截断原因。
    pub(crate) fn stop(&mut self, reason: TruncationReason) {
        self.complete = false;
        self.truncated = Some(reason);
    }
}
