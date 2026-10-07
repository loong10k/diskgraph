use diskgraph_engine::EngineError;

/// 同轮尝试两个原资源池，不因扫描 Pending 饥饿探针；来源：PF-06 原生 Rust 恢复合同。
/// 参数：seal 关闭全部原池；scan/probe 各执行一次真实 drain。
/// 返回：只有全部完成才为 true，错误保留原错误及调用方持有的 owner。
pub(crate) fn recovery_round(
    mut seal: impl FnMut() -> Result<(), EngineError>,
    mut scan: impl FnMut() -> Result<bool, EngineError>,
    mut probe: impl FnMut() -> Result<bool, EngineError>,
) -> Result<bool, EngineError> {
    seal()?;
    // 先完成同轮两次尝试，再传播首个原错误；不因 ? 提前跳过另一池。
    let scan_done = scan();
    let probe_done = probe();
    let scan_done = scan_done?;
    let probe_done = probe_done?;
    Ok(scan_done && probe_done)
}

#[cfg(test)]
mod tests;
