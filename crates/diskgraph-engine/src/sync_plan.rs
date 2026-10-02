//! 共享 Engine 的 sync_plan 职责；原调用与持锁顺序保持。

use crate::native_locator::native_path;
use crate::{Engine, EngineError};
use diskgraph_core::BusinessError;
use std::path::Path;

impl Engine {
    /// 可信内部从完整比较生成只读同步计划。
    /// 参数：from/to revision 为双侧，method 与 tolerance_seconds 为策略。
    /// 返回：计划或截断/定位失败；调用方须授权双侧。
    /// What a sync between two revisions would do, as a plan and nothing
    /// else.
    ///
    /// Reads both sides the way `compare_revisions` does, because a plan is
    /// built from the comparison and not from a second, cheaper walk: a plan
    /// that did not see every path would be a plan of what it happened to
    /// look at. It writes nothing, and there is no argument that would make
    /// it.
    /// `from_revision` is the source and `to_revision` the side being
    /// corrected. The plan builder reads `from` as the comparison's left, so
    /// swapping them is what `--from b --to a` means: a different question,
    /// not the same one asked backwards.
    pub fn sync_plan(
        &self,
        from_revision: &str,
        to_revision: &str,
        method: diskgraph_core::SyncMethod,
        tolerance_seconds: i64,
    ) -> Result<diskgraph_core::SyncPlan, EngineError> {
        let report = self.compare_revisions(from_revision, to_revision, tolerance_seconds)?;
        // 截断的比较不能生成看似完整的镜像/删除计划。
        if report.truncated.is_some() {
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }
        let left_root = self.revision_root_node(from_revision)?;
        let right_root = self.revision_root_node(to_revision)?;
        let entries = report
            .rows
            .iter()
            .map(|row| {
                let path = Path::new(&row.path);
                Ok((
                    self.revision_node_at(from_revision, path)?,
                    self.revision_node_at(to_revision, path)?,
                ))
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        let rows = report
            .rows
            .iter()
            .zip(&entries)
            .map(|(row, (left, right))| diskgraph_core::compare::Comparison {
                path: row.path.clone(),
                verdict: row.verdict.clone(),
                left: left.as_ref(),
                right: right.as_ref(),
            })
            .collect::<Vec<_>>();
        let excluded = self.index_paths_in_scope(&left_root);
        Ok(diskgraph_core::build_sync_plan(
            method,
            &rows,
            &native_path(&left_root)?,
            &native_path(&right_root)?,
            &excluded,
        ))
    }
}

impl Engine {
    /// The store's data directory, as paths relative to an indexed root, when
    /// it sits inside that root at all. A store kept outside the tree it
    /// describes - `~/.diskgraph` alongside a project - has nothing to
    /// exclude, because it is not in the comparison to begin with.
    fn index_paths_in_scope(
        &self,
        root: &diskgraph_core::DiskNode,
    ) -> Vec<diskgraph_core::PlanExclusion> {
        let Ok(root_path) = native_path(root) else {
            return Vec::new();
        };
        let data_dir = match std::fs::canonicalize(self.data_dir()) {
            Ok(path) => path,
            Err(_) => return Vec::new(),
        };
        let data_dir = data_dir.to_string_lossy().into_owned();
        let relative = match data_dir
            .strip_prefix(&root_path)
            .map(|rest| rest.trim_start_matches('/'))
        {
            Some(relative) if !relative.is_empty() => relative,
            _ => return Vec::new(),
        };
        vec![diskgraph_core::PlanExclusion {
            path: relative.to_owned(),
            reason: diskgraph_core::ExcludeReason::IndexData,
        }]
    }
}
