//! executor_transfer：既有文件操作职责的原生 Rust 实现。
use crate::cross_volume_copy::CrossVolumeCopy;
use crate::executor::Executor;
use crate::fault_point::FaultPoint;
use crate::live_item::LiveItem;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::operation_description::describe;
use crate::ops_error::OpsError;
use crate::step_result::StepResult;
use std::path::Path;

impl Executor {
    /// 演练入口按既有预算复制后移动。
    /// 参数：item、target 指定受控源对象与目标路径。
    /// 返回：复制验证与移除结果或错误；仅原有测试配置编译。
    /// Completes a cross-volume move. When this runs the verified copy is not
    /// yet published; the sequence is stage → verify → publish → remove the
    /// source, and the drill fault parks between publish and removal so a
    /// crash at that seam is observable and retry returns the parked
    /// operation instead of replaying an irreversible step (OP-05, OP-08).
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn cross_volume_move(
        &self,
        item: &LiveItem,
        target: &Path,
        fault: Option<FaultPoint>,
    ) -> Result<StepResult, OpsError> {
        self.cross_volume_move_checked(item, target, fault, &|| Ok(()))
    }

    /// 验证副本、复核实时批准后移除源。
    /// 参数：item 含已批准元数据及字节上限；target 为目标；fault 为演练位置；check_live 复核实时批准。
    /// 返回：完整复制发布和源移除的 StepResult，或显式错误、演练需关注结果。
    pub(super) fn cross_volume_move_checked(
        &self,
        item: &LiveItem,
        target: &Path,
        fault: Option<FaultPoint>,
        check_live: &dyn Fn() -> Result<(), OpsError>,
    ) -> Result<StepResult, OpsError> {
        // 暂存验证→实时批准复核→禁止覆盖发布→再次复核源→移除源，保持原有顺序。
        let transfer = CrossVolumeCopy::open(target, "move")?;
        let transferred = transfer
            .stage_and_verify_bounded(
                &item.path,
                &item.identity,
                item.bytes,
                item.approved_version.as_ref(),
                check_live,
            )
            .and_then(|copied| {
                check_live()?;
                transfer.publish().map(|()| copied)
            });
        if let Err(error) = transferred {
            transfer.discard();
            return Err(error);
        }
        if fault == Some(FaultPoint::AfterCopyBeforeSourceRemoval) {
            // The copy is complete and verified; the source is untouched. A
            // retry must reconcile this state with a human, not delete first.
            return Err(OpsError::ParkNeedsAttention(
                "the verified copy is published; the source removal did not run".into(),
            ));
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            check_live().map_err(|error| {
                OpsError::ParkNeedsAttention(format!(
                    "copy published; source removal denied: {error}"
                ))
            })?;
            let verified = transfer
                .verified_source
                .lock()
                .map_err(|_| OpsError::Stale("transfer state poisoned".into()))?;
            let source = verified
                .as_ref()
                .ok_or_else(|| OpsError::Stale("source verification is missing".into()))?;
            let _pinned = source.file.metadata()?;
            source
                .path
                .remove_verified(&source.metadata)
                .map_err(|error| {
                    OpsError::ParkNeedsAttention(format!(
                        "verified copy is published; source removal refused: {error}"
                    ))
                })?;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return Err(OpsError::Stale(
            "unsupported: verified source removal".into(),
        ));
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        Ok(StepResult {
            kind: diskgraph_store::OperationItemResult::Moved,
            detail: describe(target, item.identity.as_deref()),
            bytes: item.bytes,
            recovery_ref: None,
        })
    }
}
