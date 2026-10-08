use std::time::Duration;

/// 输出两个固定标签的累计扫描分项，仅由显式诊断分支调用，不接收路径或正文。
/// 参数：observation 为观测与编码墙钟时间，write 为含锁等待与 fence 的写入墙钟时间，nodes 为实际计费节点数。
/// 返回：无；仅输出诊断，不修改期限、授权或性能门槛，也不代表纯 SQL 或 CPU 成本。
pub(crate) fn emit(observation: Duration, write: Duration, nodes: u64) {
    for (cost, duration) in [
        ("observations_and_encoding", observation),
        ("staging_write", write),
    ] {
        eprintln!(
            "diskgraph: scan_cost={cost} nodes={nodes} total_ms={}",
            duration.as_millis()
        );
    }
}
