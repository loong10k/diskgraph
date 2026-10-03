//! executor_paths：既有文件操作职责的原生 Rust 实现。
use crate::executor::Executor;
use crate::ops_error::OpsError;
use crate::path_codec::path_of;
use crate::path_codec::unhex_key;
use diskgraph_store::Plan;
use std::path::Path;
use std::path::PathBuf;

impl Executor {
    /// 取得计划绑定范围的根。
    /// 参数：plan 为已登记计划。
    /// 返回：范围规范根或不存在范围错误。
    /// The registered root of the plan's scope; the trusted base for path
    /// revalidation.
    pub(super) fn scope_root(&self, plan: &Plan) -> Result<PathBuf, OpsError> {
        let record = self
            .engine
            .scope(&plan.scope_id)
            .map_err(|error| OpsError::Stale(format!("scope is unavailable: {error}")))?;
        path_of(&record.root)
            .ok_or_else(|| OpsError::Stale("scope root is not a native path".into()))
    }

    /// 取得范围的隔离根。
    /// 参数：无显式参数；self 持有共享 Engine。
    /// 返回：Engine 数据目录下的 quarantine 路径或错误。
    /// The same-volume holding area for quarantined objects.
    pub(super) fn quarantine_root(&self) -> Result<PathBuf, OpsError> {
        Ok(self.engine.data_dir().join("quarantine"))
    }

    /// 取得动作目标路径。
    /// 参数：plan 提供目标目录定位键；source 提供保留的叶名称。
    /// 返回：不覆盖的目标路径或错误。
    /// Where a moved or copied object lands. A plan target is a directory; the
    /// object keeps its own name inside it.
    pub(super) fn target_for(&self, plan: &Plan, source: &Path) -> Result<PathBuf, OpsError> {
        let key = plan
            .target_locator_key
            .as_ref()
            .ok_or_else(|| OpsError::Stale("plan has no target".into()))?;
        let directory =
            unhex_key(key).ok_or_else(|| OpsError::Stale("plan target is malformed".into()))?;
        let name = source
            .file_name()
            .ok_or_else(|| OpsError::Stale("source has no file name".into()))?;
        Ok(directory.join(name))
    }
}
