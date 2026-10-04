//! 固定任务目标的执行期投影；来源：Rust D42，所有拥有前准入借用原生会话，不重建 QueryReadBudget。
use crate::native_process::ProcessNativeSession;
use crate::process_evidence_target::ProcessEvidenceTarget;
use crate::process_native_error::native_error;
use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, IndexedFileEpoch, NodeKind, ProcessEvidenceJobInput};
use diskgraph_store::{SqliteSnapshotStore, StoreError};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

impl ProcessEvidenceTarget {
    /// 参数：真实引擎、不可变输入、原期限/取消/账本与实时 fence；返回：拥有前收费且归属一致的目标。
    pub(super) fn for_execution(
        engine: &Engine,
        input: &ProcessEvidenceJobInput,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
        session: &ProcessNativeSession<'_>,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        let reader =
            SqliteSnapshotStore::open_reader_until(&engine.graph_path, deadline, Some(cancel))?;
        let mut admission = |raw, entries, allocation| admit(session, raw, entries, allocation);
        let result: Result<Self, EngineError> = (|| {
            let owner = reader
                .revision_ownership_with_admission(input.base_revision_id(), &mut admission)?
                .ok_or(BusinessError::PermissionDenied)?;
            if owner.0 != input.server_id().as_str() || owner.1 != input.scope_id().as_str() {
                return Err(BusinessError::PermissionDenied.into());
            }
            check()?;
            let snapshot_id = reader
                .revision_snapshot_with_admission(input.base_revision_id(), &mut admission)?;
            let node = reader
                .node_with_admission(&snapshot_id, input.node_id(), &mut admission)?
                .ok_or(BusinessError::NotFound)?;
            if node.kind != NodeKind::File || node.read_error || !node.size_known {
                return Err(BusinessError::Unsupported.into());
            }
            let locator = reader
                .native_locator_with_admission(&snapshot_id, input.node_id(), &mut admission)?
                .and_then(|value| value.locator)
                .ok_or(BusinessError::Unsupported)?;
            let saved = reader
                .unix_observation_with_admission(&snapshot_id, input.node_id(), &mut admission)?
                .ok_or(BusinessError::Unsupported)?;
            if saved.gap.is_some() {
                return Err(BusinessError::Unsupported.into());
            }
            let observation = saved.observation.ok_or(BusinessError::Unsupported)?;
            let IndexedFileEpoch::LinuxHandle {
                device,
                inode,
                handle_bytes,
                ..
            } = observation.epoch()
            else {
                return Err(BusinessError::Unsupported.into());
            };
            // epoch 克隆及设备数字显示比较均在拥有前计入原累计分配。
            session
                .admit(0, 0, handle_bytes.len() as u64 + 20)
                .map_err(native_error)?;
            if !node
                .file_identity
                .as_ref()
                .is_some_and(|id| id.file_id == *inode && id.volume_id == device.to_string())
            {
                return Err(BusinessError::Conflict.into());
            }
            if observation.epoch() != input.indexed_epoch() {
                return Err(BusinessError::Conflict.into());
            }
            Ok(Self {
                snapshot_id,
                locator,
                epoch: observation.epoch().clone(),
            })
        })();
        check()?;
        session.check().map_err(native_error)?;
        result
    }
}

/// 参数：原账本与 Store 借用字段成本；返回：拥有前准入，真实失败类别保留在同会话中。
pub(super) fn admit(
    session: &ProcessNativeSession<'_>,
    raw: u64,
    entries: u64,
    allocation: u64,
) -> diskgraph_store::Result<()> {
    session
        .admit(raw, entries, allocation)
        .map_err(|_| StoreError::BudgetExceeded)
}
