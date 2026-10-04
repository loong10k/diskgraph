use crate::live_evidence::GitIndexedDirectory;
use crate::{Engine, EngineError};
use diskgraph_core::{
    BusinessError, GitEvidenceJobInput, QualifiedLocator, QueryBudget, QueryReadBudget,
};
use diskgraph_store::SqliteSnapshotStore;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// 固定 revision 上经窄读获得的 Git 目录与原生身份；来源：原生 Rust EC-04 / D39。
/// 不读完整快照，不把显示路径或截断 Windows ID 当作可执行身份。
pub(super) struct GitEvidenceTarget {
    pub(super) snapshot_id: String,
    pub(super) locator: QualifiedLocator,
    pub(super) identity: GitIndexedDirectory,
}

impl GitEvidenceTarget {
    /// 参数：engine/input 固定真实归属，deadline/cancel 为原任务执行边界。
    /// 返回：有限原始字段准入后的目标，或归属、定位、身份缺失/读取失败。
    pub(super) fn load(
        engine: &Engine,
        input: &GitEvidenceJobInput,
        deadline: Instant,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Self, EngineError> {
        let reader = SqliteSnapshotStore::open_reader_until(&engine.graph_path, deadline, cancel)?;
        let expected = (
            input.server_id().as_str().to_owned(),
            input.scope_id().as_str().to_owned(),
        );
        let mut reads = QueryReadBudget::new(QueryBudget::default(), deadline)?;
        let (snapshot_id, ownership) =
            reader.revision_target_with_budget(input.base_revision_id(), &mut reads)?;
        if ownership.as_ref() != Some(&expected) {
            return Err(BusinessError::PermissionDenied.into());
        }
        let node = reader
            .node_with_budget(&snapshot_id, input.node_id(), &mut reads)?
            .ok_or(BusinessError::NotFound)?;
        let locator = reader
            .native_locator_bounded(&snapshot_id, input.node_id(), &mut reads)?
            .and_then(|stored| stored.locator)
            .ok_or(BusinessError::Unsupported)?;
        // 当前宿主编码必须明确可寻址；URI 与外国 native 字节不能经 display 回退。
        locator
            .to_native_path()
            .map_err(|_| BusinessError::Unsupported)?;
        #[cfg(windows)]
        let windows = reader
            .windows_observation_bounded(&snapshot_id, input.node_id(), &mut reads)?
            .and_then(|stored| stored.observation);
        #[cfg(not(windows))]
        let windows = None;
        let identity = GitIndexedDirectory::from_node(&node, windows.as_ref())?;
        if !reads.check() {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(Self {
            snapshot_id,
            locator,
            identity,
        })
    }
}
