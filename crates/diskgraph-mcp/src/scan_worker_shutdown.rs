use diskgraph_engine::ScanWorkerRecovery;
use std::time::Duration;

/// 显式等待全部原进程回收，不在失败时丢弃恢复责任或启动隐藏后台线程。
/// 来源：原生 Rust PF-06 MCP 宿主退出合同，无 Java 对等对象。
/// 参数：recovery 在协议 catch_unwind 外持有，runner 已停止并 join；返回：全部原 wait 完成。
pub(crate) fn finish(recovery: &ScanWorkerRecovery) {
    let mut reported = false;
    loop {
        // 先关闭同一资源池准入；失败仍保留原责任，不以空槽观察允许新工作出生。
        match recovery.seal_admission().and_then(|()| recovery.drain()) {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) if !reported => {
                // 不输出镜像定位或任意 helper 错误正文；实际 owner 保留以供后续处置。
                eprintln!("scan worker recovery incomplete; shutdown retains ownership and waits");
                reported = true;
            }
            Err(_) => {}
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 显式恢复原探针进程与私有目录，失败保留责任，不启动隐藏后台线程。
/// 来源：原生 Rust PF-06，无 Java 对等对象。
/// 参数：recovery 在协议 catch 外持有且 runner 已 join；返回：所有原资源实际回收。
#[cfg(windows)]
pub(crate) fn finish_probe(recovery: &diskgraph_engine::ProbeRecovery) {
    let mut reported = false;
    loop {
        // 先关闭同一资源池准入；失败仍保留原责任，不以空槽观察允许新工作出生。
        match recovery.seal_admission().and_then(|()| recovery.drain()) {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) if !reported => {
                eprintln!("probe recovery incomplete; shutdown retains ownership and waits");
                reported = true;
            }
            Err(_) => {}
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
