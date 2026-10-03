//! 精确节点定位的存储读出值。

use diskgraph_core::QualifiedLocator;

/// 节点自身的定位与修改时间；locator=None 明确表示历史数据不可用于原生寻址。
/// 来源：DiskGraph 原生 Rust D30；无 Java 对应对象。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredNodeLocator {
    pub locator: Option<QualifiedLocator>,
    pub self_modified: Option<i64>,
}
