use serde::{Deserialize, Serialize};

/// 保存扫描的覆盖声明和实际缺口；不可读或深度受限时不能声称观察完整。
/// 来源：原生 Rust diskgraph-core::model::ScanCoverage，无 Java 对应实现。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScanCoverage {
    /// 采集者的完整性声明；消费时还须同时核验两个缺口字段。
    pub complete: bool,
    /// 已记录的不可读节点数，非零意味着未观察完整范围。
    pub unreadable_nodes: u64,
    /// 是否因深度限制遗漏范围。
    pub depth_limited: bool,
}

impl ScanCoverage {
    /// 同时核验完整声明和覆盖缺口，防止矛盾公开值被提升为可信增长或审阅候选。
    /// 参数：无，使用当前覆盖记录。返回：仅 complete=true 且无不可读及深度缺口时为 true。
    pub const fn is_complete(&self) -> bool {
        self.complete && self.unreadable_nodes == 0 && !self.depth_limited
    }
}
