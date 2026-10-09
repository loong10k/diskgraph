//! 原暂存批次 fence 与 WAL 维护的明确所有权边界。
use diskgraph_store::{ControlStore, Result, SqliteSnapshotStore};
use std::sync::MutexGuard;

/// 消费原控制 guard 后才维护已提交批次；来源：DiskGraph 原生暂存锁顺序。
/// 参数：原控制 guard、仍独占持有的图库、原 fence 结果、累计维护责任和测试观察 job。
/// 返回：原 fence 错误优先；否则保留实际维护错误，不将 PASSIVE 当作全部 WAL 已退休。
pub(crate) fn finish(
    control: MutexGuard<'_, ControlStore>,
    graph: &SqliteSnapshotStore,
    staging_result: Result<()>,
    checkpoint_due: bool,
    _job_id: &str,
) -> Result<()> {
    // 先释放原控制互斥量与已结束的 SQL fence，允许实时撤权和 keeper 续租继续。
    drop(control);
    #[cfg(test)]
    crate::scan_observation_tests::after_stage_fence(_job_id);
    let maintenance_result = if checkpoint_due {
        graph.checkpoint_after_staging()
    } else {
        Ok(())
    };
    // 控制末检失败不能遗忘已经提交图库批次的维护，也不能让维护错误覆盖原错误。
    staging_result?;
    maintenance_result
}
