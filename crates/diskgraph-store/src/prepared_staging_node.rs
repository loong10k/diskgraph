//! 当前扫描批次的不可变编码；计量与暂存复用，来源：DiskGraph 原生存储契约。
use crate::Result;
use crate::staging_node_encoding::StagingNodeEncoding;
use diskgraph_core::{DiskNode, QualifiedLocator, WindowsFileObservation, WindowsObservationGap};

/// 已完整校验的节点暂存载荷，仅保留当前配置批次；字段私有防止准入后替换内容。
/// 不包含任务授权或 fence，实际写入仍由调用方的原执行上下文检查。
/// 来源：DiskGraph 原生 Rust 扫描预算准入与原子暂存写入契约；无 Java 对等对象。
pub struct PreparedStagingNode {
    pub(crate) encoded: StagingNodeEncoding,
    pub(crate) locator: Option<QualifiedLocator>,
    pub(crate) modified: Option<i64>,
    pub(crate) gap: Option<WindowsObservationGap>,
}

impl PreparedStagingNode {
    /// 参数：node 为原节点，locator/modified 为原生定位与时间，observation/gap 为互斥观测。
    /// 返回：拥有全部实际编码字段的不可变载荷；不一致定位或观测拒绝，不访问文件系统。
    pub fn new(
        node: &DiskNode,
        locator: Option<QualifiedLocator>,
        modified: Option<i64>,
        observation: Option<&WindowsFileObservation>,
        gap: Option<WindowsObservationGap>,
    ) -> Result<Self> {
        Ok(Self {
            encoded: StagingNodeEncoding::encode_observed(
                node,
                locator.as_ref(),
                modified,
                observation,
                gap,
            )?,
            locator,
            modified,
            gap,
        })
    }

    /// 参数：无；返回：同一份实际写入字段的 checked 字节成本，不包含 SQLite 页开销。
    pub fn encoded_cost(&self) -> u64 {
        self.encoded.cost
    }
}
