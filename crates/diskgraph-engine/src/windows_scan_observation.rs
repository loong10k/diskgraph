//! Windows 扫描节点的定位资格与原生采样适配，不改变根链校验或门禁。

use crate::EngineError;
use crate::scan_node_locator::qualify_scan_locator;
use crate::scan_observation_guard::ScanObservationGuard;
use crate::windows_native_scan_root::WindowsNativeScanRoot;
use diskgraph_core::{
    BusinessError, QualifiedLocator, ScanSettings, WindowsFileObservation, WindowsObservationGap,
};
use diskgraph_disktree::NodeV2;

/// 一个扫描节点的原始定位及原生观测；来源：DiskGraph Windows 暂存采样。
pub(super) type WindowsScanObservation = (
    QualifiedLocator,
    (
        Option<WindowsFileObservation>,
        Option<WindowsObservationGap>,
    ),
);

/// 参数：batch/root/settings 为当前批次及根租约，guard_factory 创建同代独立门禁。
/// 返回：按原顺序收齐的采样或原失败；局部门禁不跨线程共享，keeper原错误仍由主执行器消费。
pub(super) fn observe_batch<'a>(
    batch: &[NodeV2],
    root: &WindowsNativeScanRoot,
    settings: &ScanSettings,
    guard_factory: &(impl Fn() -> ScanObservationGuard<'a> + Sync),
) -> Result<Vec<WindowsScanObservation>, EngineError> {
    crate::scan_observation_batch::collect_ordered(batch, &|chunk| {
        let guard = guard_factory();
        chunk
            .iter()
            .map(|node| observe_node(root, node, settings, &|| guard.check()))
            .collect()
    })
}

/// 参数：root 为原根租约，node/settings 来自本次扫描，check 保留当前采样门禁。
/// 返回：原定位及原生观测，定位错误和原采样错误直接传播。
pub(super) fn observe_node(
    root: &WindowsNativeScanRoot,
    node: &NodeV2,
    settings: &ScanSettings,
    check: &dyn Fn() -> Result<(), EngineError>,
) -> Result<WindowsScanObservation, EngineError> {
    check()?;
    let locator = qualify_scan_locator(node)?;
    let observed = root.observe(
        &locator
            .to_native_path()
            .map_err(|_| BusinessError::Unsupported)?,
        &node.v1,
        node.self_modified,
        settings,
        check,
    )?;
    Ok((locator, observed))
}
