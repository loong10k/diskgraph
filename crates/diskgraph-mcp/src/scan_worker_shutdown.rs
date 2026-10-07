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

/// 退休同次 MCP 启动的全部原材料；Pending 或错误均保留原 owner。
/// 参数：parts 为 runner join 后的原 Engine/Recovery/ACTIVE。返回：实际完成且 CLEAN 已同步。
/// 当前仍是无限兼容等待，不声明有限前台退出。
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn finish_original(mut parts: diskgraph_engine::SupervisorParts) {
    let mut owner = loop {
        match diskgraph_engine::SupervisorOwner::bind(
            parts,
            std::time::Instant::now() + Duration::from_millis(50),
        ) {
            Ok(owner) => break owner,
            Err(original) => parts = original,
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut reported = false;
    loop {
        match owner.poll_retirement(std::time::Instant::now() + Duration::from_millis(50)) {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) if !reported => {
                eprintln!("native recovery incomplete; original MCP owner retains responsibility");
                reported = true;
            }
            Err(_) => {}
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
