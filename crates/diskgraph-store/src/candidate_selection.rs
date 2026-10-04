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
    /// 是否在同次预算内成功读取并验证覆盖头；false 表示未观测，不能解释为真实覆盖缺口。
    /// 来源：DiskGraph 原生 Rust Q-09 覆盖诊断；无 Java 对应字段。
    pub coverage_observed: bool,
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
            coverage_observed: true,
            complete: coverage_complete,
            truncated: None,
        }
    }

    /// 在覆盖头尚未取得时保留原到期部分结果，不伪造已观测到的覆盖缺口。
    /// 参数：target_bytes 为原目标。返回：空候选、完整目标缺口及明确未观测的 Deadline 状态。
    pub(crate) fn unobserved_deadline(target_bytes: u64) -> Self {
        let mut result = Self::empty(target_bytes, false);
        result.coverage_observed = false;
        result.stop(TruncationReason::Deadline);
        result
    }

    /// 准备有界审阅候选、目标缺口和真实截断状态。
    /// 参数：reason：预算截断原因。
    /// 返回：无返回值；候选变为不完整并记录真实截断原因。
    pub(crate) fn stop(&mut self, reason: TruncationReason) {
        self.complete = false;
        self.truncated = Some(reason);
    }
}
