//! 双侧有序历史比较，共用原始期限、实际解码账本与完整报告编码额度。

use crate::native_locator::native_path;
use crate::{CompareRow, ComparisonReport, Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, PrincipalId, QueryBudget, QueryReadBudget, TruncationReason,
    measure_json_bounded, observed_node_size, query_deadline,
};
use diskgraph_store::{SqliteSnapshotStore, StoreError};
use std::time::Instant;

impl Engine {
    /// 可信内部以默认预算有序合并两份历史。
    /// 参数：left/right revision 与 tolerance_seconds 指定比较，调用方须先授权双侧。
    /// 返回：兼容报告；截断统计只描述已经完成比较的条目。
    pub fn compare_revisions(
        &self,
        left: &str,
        right: &str,
        tolerance_seconds: i64,
    ) -> Result<ComparisonReport, EngineError> {
        self.compare_revisions_bounded(left, right, tolerance_seconds, QueryBudget::default())
    }

    /// 可信有序合并在首次连接前建立期限。
    /// 参数：双侧 revision、时间容差与 budget 指定约束。
    /// 返回：真实 JSON 编码额度内的报告或最小诊断也无法容纳的预算错误。
    pub fn compare_revisions_bounded(
        &self,
        left: &str,
        right: &str,
        tolerance: i64,
        budget: QueryBudget,
    ) -> Result<ComparisonReport, EngineError> {
        self.compare_revisions_bounded_until(
            left,
            right,
            tolerance,
            budget,
            query_deadline(budget)?,
        )
    }

    /// 可信历史比较复用两连接上的同一绝对期限。
    /// 参数：left/right/tolerance/budget 为固定比较，deadline 为请求起点派生期限。
    /// 返回：预算报告；调用方负责双侧首末授权，不重新计时或加载完整树。
    pub fn compare_revisions_bounded_until(
        &self,
        left: &str,
        right: &str,
        tolerance: i64,
        budget: QueryBudget,
        deadline: Instant,
    ) -> Result<ComparisonReport, EngineError> {
        let left_reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let right_reader =
            SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let left_snapshot = left_reader.revision(left)?.snapshot_id;
        let right_snapshot = right_reader.revision(right)?.snapshot_id;
        let mut ledger = QueryReadBudget::new(budget, deadline)?;
        let mut report = Self::compare_on_readers(
            &left_reader,
            &left_snapshot,
            left,
            &right_reader,
            &right_snapshot,
            right,
            tolerance,
            budget,
            &mut ledger,
        )?;
        Self::finish_comparison(&mut report, budget, Instant::now() >= deadline)?;
        Ok(report)
    }

    /// 双侧历史先按真实归属授权，完整报告编码后重查双侧权限。
    /// 参数：left/right/tolerance/budget 为比较，principal/authorizer 为身份，deadline 不重置。
    /// 返回：允许的完整或准确部分报告；撤权拒绝部分数据。
    #[allow(clippy::too_many_arguments)] // 双侧比较与请求身份、预算各自必需。
    pub fn compare_revisions_until(
        &self,
        left: &str,
        right: &str,
        tolerance: i64,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<ComparisonReport, EngineError> {
        self.with_history_readers_until(
            left,
            right,
            principal,
            authorizer,
            deadline,
            |left_reader, left_snapshot, right_reader, right_snapshot| {
                let mut ledger = QueryReadBudget::new(budget, deadline)?;
                Self::compare_on_readers(
                    left_reader,
                    left_snapshot,
                    left,
                    right_reader,
                    right_snapshot,
                    right,
                    tolerance,
                    budget,
                    &mut ledger,
                )
            },
            |report, expired| Self::finish_comparison(report, budget, expired),
        )
    }

    /// 在已有两个 reader 和同一累计账本上比较。
    /// 参数：双侧 reader/snapshot/revision、时间容差、响应预算与实际读取 ledger。
    /// 返回：报告；根与各方向当前行均计费，额度外只做存在探针。
    #[allow(clippy::too_many_arguments)] // 准备好的双侧连接与各自标识不能隐式混用。
    pub(super) fn compare_on_readers(
        left: &SqliteSnapshotStore,
        left_snapshot: &str,
        left_revision: &str,
        right: &SqliteSnapshotStore,
        right_snapshot: &str,
        right_revision: &str,
        tolerance: i64,
        budget: QueryBudget,
        ledger: &mut QueryReadBudget,
    ) -> Result<ComparisonReport, EngineError> {
        let left_root = left
            .root_node_with_budget(left_snapshot, ledger)?
            .ok_or(BusinessError::NotFound)?;
        let right_root = right
            .root_node_with_budget(right_snapshot, ledger)?
            .ok_or(BusinessError::NotFound)?;
        let left_path = native_path(&left_root)?;
        let right_path = native_path(&right_root)?;
        let mut report = ComparisonReport {
            left_revision: left_revision.to_owned(),
            right_revision: right_revision.to_owned(),
            left_root: left_root.locator,
            right_root: right_root.locator,
            left_nodes: 0,
            right_nodes: 0,
            node_counts_complete: true,
            rows: Vec::new(),
            summary: diskgraph_core::Summary::default(),
            truncated: None,
        };
        for (reader, snapshot, count) in [
            (left, left_snapshot, &mut report.left_nodes),
            (right, right_snapshot, &mut report.right_nodes),
        ] {
            if !ledger.check() {
                report.node_counts_complete = false;
                break;
            }
            match reader.node_count(snapshot) {
                Ok(value) => {
                    *count = usize::try_from(value).map_err(|_| StoreError::IntegerOverflow)?
                }
                Err(error) if error.is_interrupted() => {
                    ledger.stop(TruncationReason::Deadline);
                    report.node_counts_complete = false;
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
        // 原始字段准入独立于响应额度；这里只按真实 wire 计量，不重复扣原始字段。
        let mut bytes = measure_json_bounded(&report.to_json(None), budget.max_response_bytes)
            .map_err(StoreError::from)?
            .ok_or(BusinessError::BudgetExceeded)?;
        let result = left.with_ordered_nodes_bounded(left_snapshot, &left_path, |left_cursor| {
            right.with_ordered_nodes_bounded(right_snapshot, &right_path, |right_cursor| {
                let mut current_left = left_cursor.next(ledger)?;
                let mut current_right = right_cursor.next(ledger)?;
                while current_left.is_some() || current_right.is_some() {
                    if !ledger.check() {
                        break;
                    }
                    let ordering = match (&current_left, &current_right) {
                        (Some(left), Some(right)) => left.0.cmp(&right.0),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        _ => break,
                    };
                    let on_left = if ordering != std::cmp::Ordering::Greater {
                        current_left.take()
                    } else {
                        None
                    };
                    let on_right = if ordering != std::cmp::Ordering::Less {
                        current_right.take()
                    } else {
                        None
                    };
                    let source = on_left
                        .as_ref()
                        .or(on_right.as_ref())
                        .expect("one ordered row exists");
                    let verdict = match (&on_left, &on_right) {
                        (Some(left), Some(right)) => {
                            diskgraph_core::compare::compare_entry(&left.1, &right.1, tolerance)
                        }
                        (Some(_), None) => diskgraph_core::Verdict::LeftOnly,
                        _ => diskgraph_core::Verdict::RightOnly,
                    };
                    let row = CompareRow {
                        path: source.0.clone(),
                        verdict,
                        left_bytes: on_left
                            .as_ref()
                            .and_then(|node| observed_node_size(&node.1)),
                        right_bytes: on_right
                            .as_ref()
                            .and_then(|node| observed_node_size(&node.1)),
                        is_file: source.1.kind == diskgraph_core::NodeKind::File,
                        digests: None,
                    };
                    let available = budget.max_response_bytes.saturating_sub(bytes);
                    let size = measure_json_bounded(&row.to_json(), available)?
                        .and_then(|size| size.checked_add(usize::from(!report.rows.is_empty())));
                    let Some(next_bytes) = size
                        .and_then(|size| bytes.checked_add(size))
                        .filter(|size| *size <= budget.max_response_bytes)
                    else {
                        ledger.stop(TruncationReason::ByteLimit);
                        break;
                    };
                    bytes = next_bytes;
                    report.rows.push(row);
                    // cursor 在零余量时只检查存在，不能拥有/解码下一条损坏记录。
                    if ordering != std::cmp::Ordering::Greater {
                        current_left = left_cursor.next(ledger)?;
                    }
                    if ordering != std::cmp::Ordering::Less {
                        current_right = right_cursor.next(ledger)?;
                    }
                }
                Ok(())
            })
        });
        match result {
            Ok(()) => {}
            Err(StoreError::BudgetExceeded) => {}
            Err(error) if error.is_interrupted() => ledger.stop(TruncationReason::Deadline),
            Err(error) => return Err(error.into()),
        }
        ledger.check();
        report.truncated = ledger.stopped().map(history_reason);
        Self::finish_comparison(&mut report, budget, Instant::now() >= ledger.deadline())?;
        Ok(report)
    }

    /// 按完整报告最终编码尺寸截页，并重算仅涵盖保留条目的统计。
    /// 参数：report 为已读取前缀，budget 为真实报告 cap，expired 表示整次期限已到。
    /// 返回：编码可容纳；最小报告仍超额明确 BudgetExceeded。
    pub(super) fn finish_comparison(
        report: &mut ComparisonReport,
        budget: QueryBudget,
        expired: bool,
    ) -> Result<(), EngineError> {
        if expired {
            report.truncated = Some("deadline");
        }
        loop {
            report.summary = diskgraph_core::Summary::default();
            for row in &report.rows {
                match &row.verdict {
                    diskgraph_core::Verdict::LeftOnly => report.summary.left_only += 1,
                    diskgraph_core::Verdict::RightOnly => report.summary.right_only += 1,
                    diskgraph_core::Verdict::Same { .. } => report.summary.same += 1,
                    diskgraph_core::Verdict::Different { reason } => {
                        report.summary.different += 1;
                        if *reason == diskgraph_core::DifferentReason::UnknownSize {
                            report.summary.unknown += 1;
                        }
                    }
                }
            }
            if measure_json_bounded(&report.to_json(None), budget.max_response_bytes)
                .map_err(StoreError::from)?
                .is_some()
            {
                return Ok(());
            }
            if report.rows.pop().is_none() {
                return Err(BusinessError::BudgetExceeded.into());
            }
            if !expired {
                report.truncated = Some("response_byte_limit");
            }
        }
    }
}

fn history_reason(reason: TruncationReason) -> &'static str {
    match reason {
        TruncationReason::Deadline => "deadline",
        TruncationReason::ByteLimit => "response_byte_limit",
        TruncationReason::NodeLimit => "node_limit",
        TruncationReason::EdgeLimit => "edge_limit",
        TruncationReason::DepthLimit => "depth_limit",
    }
}
