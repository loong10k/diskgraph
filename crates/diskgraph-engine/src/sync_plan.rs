//! 共享 Engine 的 sync_plan 职责；原调用与持锁顺序保持。

use crate::native_locator::native_path;
use crate::{Engine, EngineError};
use diskgraph_core::BusinessError;
use diskgraph_core::{Authorizer, PrincipalId, QueryBudget, QueryReadBudget, measure_json_bounded};
use diskgraph_store::StoreError;
use std::path::Path;
use std::time::Instant;

impl Engine {
    /// 在双侧授权 reader 与实际累计账本上生成完整只读同步计划。
    /// 参数：from/to/method/tolerance/budget 为计划范围，principal/authorizer/deadline 为请求。
    /// 返回：完整且编码可容纳的计划；重读耗尽额度或到期明确失败，不返回局部计划。
    #[allow(clippy::too_many_arguments)] // 双侧同步策略与请求身份、预算均为必需上下文。
    pub fn sync_plan_until(
        &self,
        from: &str,
        to: &str,
        method: diskgraph_core::SyncMethod,
        tolerance: i64,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<diskgraph_core::SyncPlan, EngineError> {
        self.with_history_readers_until(
            from,
            to,
            principal,
            authorizer,
            deadline,
            |left, left_snapshot, right, right_snapshot| {
                let mut ledger = QueryReadBudget::new(budget, deadline)?;
                let report = Self::compare_on_readers(
                    left,
                    left_snapshot,
                    from,
                    right,
                    right_snapshot,
                    to,
                    tolerance,
                    budget,
                    &mut ledger,
                )?;
                if report.truncated.is_some() {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                let left_root = left
                    .root_node_with_budget(left_snapshot, &mut ledger)?
                    .ok_or(BusinessError::NotFound)?;
                let right_root = right
                    .root_node_with_budget(right_snapshot, &mut ledger)?
                    .ok_or(BusinessError::NotFound)?;
                // 计划需要完整节点：每次窄重读计费，绝不重新开连接或重置预算。
                let entries = report
                    .rows
                    .iter()
                    .map(|row| {
                        Ok((
                            Self::history_node_at(
                                left,
                                left_snapshot,
                                Path::new(&row.path),
                                &mut ledger,
                            )?,
                            Self::history_node_at(
                                right,
                                right_snapshot,
                                Path::new(&row.path),
                                &mut ledger,
                            )?,
                        ))
                    })
                    .collect::<Result<Vec<_>, EngineError>>()?;
                if !ledger.check() {
                    return Err(BusinessError::Timeout.into());
                }
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
                Ok(diskgraph_core::build_sync_plan(
                    method,
                    &rows,
                    &native_path(&left_root)?,
                    &native_path(&right_root)?,
                    &self.index_paths_in_scope(&left_root),
                ))
            },
            |plan, expired| {
                if expired {
                    return Err(BusinessError::Timeout.into());
                }
                if measure_json_bounded(plan, budget.max_response_bytes)
                    .map_err(StoreError::from)?
                    .is_none()
                {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                Ok(())
            },
        )
    }

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
