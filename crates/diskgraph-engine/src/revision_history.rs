//! 历史增长与变化使用双侧共享 reader、原始期限和读取账本。

use crate::{Engine, EngineError, RevisionGrowth};
use diskgraph_core::{
    Authorizer, BusinessError, DiskSnapshot, PrincipalId, QueryBudget, QueryReadBudget,
    comparable_growth_delta, measure_json_bounded, query_deadline,
};
use diskgraph_store::{SqliteSnapshotStore, StoreError};
use std::path::Path;
use std::time::Instant;

impl Engine {
    /// 可信读取可比较历史中同一路径的增长。
    /// 参数：before/after revision 与 relative 须由调用方先授权。
    /// 返回：可比较的前后节点；不兼容、缺一侧、类型不同或大小未知/读取失败时 None，准备计入默认期限。
    /// 旧历史未绑定归属或属于其他服务器时拒绝，不能凭显示根推断命名空间。
    pub fn growth_between(
        &self,
        before: &str,
        after: &str,
        relative: &Path,
    ) -> Result<Option<RevisionGrowth>, EngineError> {
        let budget = QueryBudget::default();
        let deadline = query_deadline(budget)?;
        let left = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let right = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let left_snapshot = left.revision(before)?.snapshot_id;
        let right_snapshot = right.revision(after)?.snapshot_id;
        let mut ledger = QueryReadBudget::new(budget, deadline)?;
        let value = if self.history_revisions_share_namespace(&left, before, &right, after)? {
            Self::growth_on_readers(
                &left,
                &left_snapshot,
                &right,
                &right_snapshot,
                relative,
                &mut ledger,
            )?
        } else {
            None
        };
        finish_growth(&value, budget, Instant::now() >= deadline)?;
        Ok(value)
    }

    /// 增长准备、节点查找和末段授权共用整次绝对期限。
    /// 参数：before/after/relative/budget 为范围，principal/authorizer 为请求身份，deadline 不重置。
    /// 返回：可比较增长或 None；到期与撤权不能被解释为正常不可比较。
    #[allow(clippy::too_many_arguments)] // 双侧历史、相对路径与请求预算均为必需上下文。
    pub fn growth_between_until(
        &self,
        before: &str,
        after: &str,
        relative: &Path,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<Option<RevisionGrowth>, EngineError> {
        self.with_history_readers_until(
            before,
            after,
            principal,
            authorizer,
            deadline,
            |left, left_snapshot, right, right_snapshot| {
                let mut ledger = QueryReadBudget::new(budget, deadline)?;
                if !self.history_revisions_share_namespace(left, before, right, after)? {
                    // 这里只结束 consumer；外层仍执行预算编码及双侧终态授权。
                    return Ok(None);
                }
                Self::growth_on_readers(
                    left,
                    left_snapshot,
                    right,
                    right_snapshot,
                    relative,
                    &mut ledger,
                )
            },
            |value, expired| finish_growth(value, budget, expired),
        )
    }

    /// 使用已有 reader 查找可比较增长，元数据/每个路径组件均从同一账本准入。
    /// 参数：双侧 reader/snapshot、relative 与 ledger 为固定查询上下文。
    /// 返回：可比较节点差值或 None；真实存储与预算错误保留。
    pub(super) fn growth_on_readers(
        left: &SqliteSnapshotStore,
        left_snapshot: &str,
        right: &SqliteSnapshotStore,
        right_snapshot: &str,
        relative: &Path,
        ledger: &mut QueryReadBudget,
    ) -> Result<Option<RevisionGrowth>, EngineError> {
        let before = left.snapshot_with_budget(left_snapshot, ledger)?;
        let after = right.snapshot_with_budget(right_snapshot, ledger)?;
        if incompatibility(&before, &after).is_some() {
            return Ok(None);
        }
        let (Some(before), Some(after)) = (
            Self::history_node_at(left, left_snapshot, relative, ledger)?,
            Self::history_node_at(right, right_snapshot, relative, ledger)?,
        ) else {
            return Ok(None);
        };
        let Some(delta_bytes) = comparable_growth_delta(&before, &after) else {
            return Ok(None);
        };
        Ok(Some(RevisionGrowth {
            delta_bytes,
            before,
            after,
        }))
    }

    /// 可信生成有界历史变化的兼容 wire 结果。
    /// 参数：before/after 为已由调用方授权历史标识。
    /// 返回：变化 JSON；截断明确标为部分统计，全部准备沿用默认同一期限。
    /// 实际 scope 不同时保留 different_root 并补充 scope_changed；缺失或无效归属仍报错。
    pub fn revision_changes(
        &self,
        before: &str,
        after: &str,
    ) -> Result<serde_json::Value, EngineError> {
        let budget = QueryBudget::default();
        let deadline = query_deadline(budget)?;
        let left = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let right = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let left_snapshot = left.revision(before)?.snapshot_id;
        let right_snapshot = right.revision(after)?.snapshot_id;
        let mut value = if self.history_revisions_share_namespace(&left, before, &right, after)? {
            Self::changes_on_readers(
                &left,
                &left_snapshot,
                before,
                &right,
                &right_snapshot,
                after,
                budget,
                deadline,
            )?
        } else {
            scope_incompatible_changes()
        };
        finish_changes(&mut value, budget, Instant::now() >= deadline)?;
        Ok(value)
    }

    /// 历史变化在编码之后复检双側实际授权及整次期限。
    /// 参数：before/after/budget 为固定比较，principal/authorizer/deadline 为同请求身份与期限。
    /// 返回：兼容变化 JSON；过期部分统计有明确 deadline，撤权拒绝数据。
    pub fn revision_changes_until(
        &self,
        before: &str,
        after: &str,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<serde_json::Value, EngineError> {
        self.with_history_readers_until(
            before,
            after,
            principal,
            authorizer,
            deadline,
            |left, left_snapshot, right, right_snapshot| {
                // 即使无需读取节点，不可比结果也不能跳过原 typed 预算参数校验。
                budget.validated()?;
                if !self.history_revisions_share_namespace(left, before, right, after)? {
                    // 不可比仍走 finish_changes 和已有末段授权，不能隐藏拒权或超时。
                    return Ok(scope_incompatible_changes());
                }
                Self::changes_on_readers(
                    left,
                    left_snapshot,
                    before,
                    right,
                    right_snapshot,
                    after,
                    budget,
                    deadline,
                )
            },
            |value, expired| finish_changes(value, budget, expired),
        )
    }

    /// 用双侧固定 reader 与同一账本产生变化前缀。
    /// 参数：双侧 reader/snapshot/revision 及 budget/deadline 为固定上下文。
    /// 返回：兼容性诊断或局部统计；真实读取成本不因进入 merge 重置。
    #[allow(clippy::too_many_arguments)] // 双侧已解析标识与预算不能隐式替换。
    pub(super) fn changes_on_readers(
        left: &SqliteSnapshotStore,
        left_snapshot: &str,
        before: &str,
        right: &SqliteSnapshotStore,
        right_snapshot: &str,
        after: &str,
        budget: QueryBudget,
        deadline: Instant,
    ) -> Result<serde_json::Value, EngineError> {
        let mut ledger = QueryReadBudget::new(budget, deadline)?;
        let previous = left.snapshot_with_budget(left_snapshot, &mut ledger)?;
        let current = right.snapshot_with_budget(right_snapshot, &mut ledger)?;
        if let Some(reason) = incompatibility(&previous, &current) {
            return Ok(
                serde_json::json!({"incompatible":reason,"added":0,"removed":0,"size_changed":0,"complete":true,"summary_is_partial":false}),
            );
        }
        let report = Self::compare_on_readers(
            left,
            left_snapshot,
            before,
            right,
            right_snapshot,
            after,
            0,
            budget,
            &mut ledger,
        )?;
        // 类型替换与未知大小不能贡献数值增长；目录 Contents 仍可携带有效大小变化。
        let size_changed = report
            .rows
            .iter()
            .filter(|row| {
                matches!(
                    row.verdict,
                    diskgraph_core::Verdict::Different {
                        reason: diskgraph_core::DifferentReason::Size
                            | diskgraph_core::DifferentReason::Contents
                    }
                ) && row.left_bytes.is_some()
                    && row.right_bytes.is_some()
                    && row.left_bytes != row.right_bytes
            })
            .count();
        Ok(
            serde_json::json!({"incompatible":null,"added":report.summary.right_only,"removed":report.summary.left_only,"size_changed":size_changed,"complete":report.truncated.is_none(),"summary_is_partial":report.truncated.is_some(),"truncation_reason":report.truncated}),
        )
    }
}

fn scope_incompatible_changes() -> serde_json::Value {
    serde_json::json!({
        "incompatible": "different_root",
        "scope_changed": true,
        "added": 0,
        "removed": 0,
        "size_changed": 0,
        "complete": true,
        "summary_is_partial": false
    })
}

fn incompatibility(before: &DiskSnapshot, after: &DiskSnapshot) -> Option<&'static str> {
    if before.root != after.root {
        Some("different_root")
    } else if before.volume_id.is_none() || after.volume_id.is_none() {
        Some("unknown_volume")
    } else if before.volume_id != after.volume_id {
        Some("different_volume")
    } else if before.settings != after.settings {
        Some("different_settings")
    } else if before.captured_at_unix_ms > after.captured_at_unix_ms {
        Some("out_of_order")
    } else if !before.coverage.complete || !after.coverage.complete {
        Some("incomplete_coverage")
    } else {
        None
    }
}

fn finish_growth(
    value: &Option<RevisionGrowth>,
    budget: QueryBudget,
    expired: bool,
) -> Result<(), EngineError> {
    if expired {
        return Err(BusinessError::Timeout.into());
    }
    let value=value.as_ref().map(|growth|serde_json::json!({"before":growth.before,"after":growth.after,"delta_bytes":growth.delta_bytes.to_string()}));
    if measure_json_bounded(&value, budget.max_response_bytes)
        .map_err(StoreError::from)?
        .is_none()
    {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}

fn finish_changes(
    value: &mut serde_json::Value,
    budget: QueryBudget,
    expired: bool,
) -> Result<(), EngineError> {
    if expired {
        value["complete"] = serde_json::json!(false);
        value["summary_is_partial"] = serde_json::json!(true);
        value["truncation_reason"] = serde_json::json!("deadline");
    }
    if measure_json_bounded(value, budget.max_response_bytes)
        .map_err(StoreError::from)?
        .is_none()
    {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}
