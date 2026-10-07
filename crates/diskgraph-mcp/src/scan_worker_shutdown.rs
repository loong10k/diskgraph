use diskgraph_engine::{EngineError, ScanWorkerRecovery};
use std::time::Duration;

/// 显式等待原扫描及探针资源，不丢弃恢复责任，不启动隐藏后台线程。
/// 来源：原生 Rust PF-06 MCP 宿主退出合同，无 Java 对等对象。
/// 参数：recovery 为可选原扫描责任，Windows probe 为原探针责任；runner 须已停止并 join。
/// 返回：全部原资源实际回收；无限兼容等待不能作为有限前端退出证明。
pub(crate) fn finish(
    recovery: Option<&ScanWorkerRecovery>,
    #[cfg(windows)] probe: &diskgraph_engine::ProbeRecovery,
) {
    let mut reported = false;
    loop {
        match round(
            recovery,
            #[cfg(windows)]
            probe,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) if !reported => {
                eprintln!("native recovery incomplete; shutdown retains ownership and waits");
                reported = true;
            }
            Err(_) => {}
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn round(
    recovery: Option<&ScanWorkerRecovery>,
    #[cfg(windows)] probe: &diskgraph_engine::ProbeRecovery,
) -> Result<bool, EngineError> {
    let scan_sealed = recovery.map_or(Ok(()), |r| r.seal_admission());
    // 先尝试关闭全部原池，任一关闭失败也不能留下另一池继续接受新工作。
    #[cfg(windows)]
    let probe_sealed = probe.seal_admission();
    scan_sealed?;
    #[cfg(windows)]
    probe_sealed?;
    let scan_done = recovery.map_or(Ok(true), |r| r.drain());
    #[cfg(windows)]
    let probe_done = probe.drain();
    let done = scan_done?;
    #[cfg(windows)]
    let done = {
        let probe_done = probe_done?;
        done && probe_done
    };
    Ok(done)
}

#[cfg(test)]
mod tests;
