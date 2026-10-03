//! 节点完整 Windows 原生观测的存储读取值。

use diskgraph_core::{WindowsFileObservation, WindowsObservationGap};

/// 完整原生观测或固定缺失原因；历史全空记录返回 NotCaptured。
/// 来源：DiskGraph 原生 Rust D31；无 Java 对应对象。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredWindowsObservation {
    pub observation: Option<WindowsFileObservation>,
    pub gap: Option<WindowsObservationGap>,
}
