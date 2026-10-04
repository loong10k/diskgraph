use crate::process_evidence_entry::admit_entry_raw;
use crate::{Engine, EngineError};
use diskgraph_core::{
    BusinessError, IndexedFileEpoch, NodeKind, QualifiedLocator, QueryReadBudget, ScopeId,
};
use diskgraph_store::SqliteSnapshotStore;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// 固定 revision 上的普通文件与扫描强 epoch；来源：Rust D42 / Q08，不打开源或补旧身份。
pub(super) struct ProcessEvidenceTarget {
    pub(super) snapshot_id: String,
    pub(super) locator: QualifiedLocator,
    pub(super) epoch: IndexedFileEpoch,
}
impl ProcessEvidenceTarget {
    /// 参数：实际scope/固定revision/node、入口原读取账本/取消和当前授权检查；返回：同一raw账本读取的可执行目标。
    /// 字节准入在拥有前完成，consumer失败仍先执行权限末检；未知平台和旧记录不伪造能力。
    pub(super) fn load(
        engine: &Engine,
        scope: &ScopeId,
        revision: &str,
        node_id: u64,
        reads: &mut QueryReadBudget,
        cancel: Option<Arc<AtomicBool>>,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        let deadline = reads.deadline();
        let reader = SqliteSnapshotStore::open_reader_until(&engine.graph_path, deadline, cancel)?;
        let owner = reader
            .revision_ownership_with_budget(revision, reads)?
            .ok_or(BusinessError::PermissionDenied)?;
        #[cfg(test)]
        crate::process_entry_budget_tests::after_owner_read();
        let server = {
            let control = engine
                .try_control_store()?
                .ok_or(BusinessError::BudgetExceeded)?;
            control.with_read_deadline(deadline, |control| {
                control
                    .existing_server_id_with_admission(&mut |raw, _, _| admit_entry_raw(reads, raw))
            })?
        };
        if owner.0 != server.as_str() || owner.1 != scope.as_str() {
            return Err(BusinessError::PermissionDenied.into());
        }
        check()?;
        let result = (|| {
            let snapshot_id = reader.revision_snapshot_with_budget(revision, reads)?;
            let node = reader
                .node_with_budget(&snapshot_id, node_id, reads)?
                .ok_or(BusinessError::NotFound)?;
            if node.kind != NodeKind::File || node.read_error || !node.size_known {
                return Err(BusinessError::Unsupported.into());
            }
            // Mac generation=0 / Windows 名称绑定尚未证明，不能仅有方法标签就入队。
            if !cfg!(target_os = "linux") {
                return Err(BusinessError::Unsupported.into());
            }
            let locator = reader
                .native_locator_bounded(&snapshot_id, node_id, reads)?
                .and_then(|v| v.locator)
                .ok_or(BusinessError::Unsupported)?;
            locator
                .to_native_path()
                .map_err(|_| BusinessError::Unsupported)?;
            let saved = reader
                .unix_observation_bounded(&snapshot_id, node_id, reads)?
                .ok_or(BusinessError::Unsupported)?;
            if saved.gap.is_some() {
                return Err(BusinessError::Unsupported.into());
            }
            let observation = saved.observation.ok_or(BusinessError::Unsupported)?;
            let epoch = observation.epoch().clone();
            let IndexedFileEpoch::LinuxHandle { device, inode, .. } = &epoch else {
                return Err(BusinessError::Unsupported.into());
            };
            if !node
                .file_identity
                .as_ref()
                .is_some_and(|id| id.file_id == *inode && id.volume_id == device.to_string())
            {
                return Err(BusinessError::Conflict.into());
            }
            if !reads.check() {
                return Err(BusinessError::BudgetExceeded.into());
            }
            Ok(Self {
                snapshot_id,
                locator,
                epoch,
            })
        })();
        check()?;
        if !reads.check() {
            return Err(BusinessError::BudgetExceeded.into());
        }
        result
    }
}
