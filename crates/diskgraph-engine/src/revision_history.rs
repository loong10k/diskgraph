//! 共享 Engine 的 revision_history 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError, RevisionGrowth};
use std::path::Path;

impl Engine {
    /// 可信读取可比较历史中同一路径的增长。
    /// 参数：before/after revision 与 relative 须先授权。
    /// 返回：可选前后节点和变化；不兼容或缺一侧时 None。
    /// How much one path grew between two published revisions.
    ///
    /// Reads the two nodes it needs and nothing else: the previous answer had
    /// to materialize both revisions, which on a four-million-node index meant
    /// four million nodes in memory to compare two of them. The comparability
    /// rules are the ones the in-memory version applies — a differing root,
    /// volume, or scan setting, an unknown volume, an earlier "after", or an
    /// incomplete scan on either side all answer "not comparable" rather than
    /// a number, because a size delta across incomparable scans is a fiction.
    pub fn growth_between(
        &self,
        before_revision: &str,
        after_revision: &str,
        relative: &Path,
    ) -> Result<Option<RevisionGrowth>, EngineError> {
        // The comparability check reads only metadata, so the store lock is
        // released before the node lookups: those take the same lock, and
        // holding it across them would wait on this thread's own guard.
        {
            let graph = self.revision_reader()?;
            let before_record = graph.revision(before_revision)?;
            let after_record = graph.revision(after_revision)?;
            let before_snapshot = graph.snapshot(&before_record.snapshot_id)?;
            let after_snapshot = graph.snapshot(&after_record.snapshot_id)?;
            if before_snapshot.root != after_snapshot.root
                || before_snapshot.volume_id != after_snapshot.volume_id
                || after_snapshot.volume_id.is_none()
                || before_snapshot.settings != after_snapshot.settings
                || before_snapshot.captured_at_unix_ms > after_snapshot.captured_at_unix_ms
                || !before_snapshot.coverage.complete
                || !after_snapshot.coverage.complete
            {
                return Ok(None);
            }
        }
        let (Some(before), Some(after)) = (
            self.revision_node_at(before_revision, relative)?,
            self.revision_node_at(after_revision, relative)?,
        ) else {
            // A path absent from one side is a removal or an addition, never
            // a growth. Renames are not inferred, same as before.
            return Ok(None);
        };
        let delta_bytes = i128::from(after.subtree_bytes) - i128::from(before.subtree_bytes);
        Ok(Some(RevisionGrowth {
            before,
            after,
            delta_bytes,
        }))
    }
}

impl Engine {
    /// 可信生成有界历史变化的兼容 wire 结果。
    /// 参数：before/after 为已授权历史标识。
    /// 返回：变化 JSON，截断明确标为部分统计。
    /// 有界历史变化，字段兼容；截断统计明确标为部分结果。调用方须先授权两个 revision。
    pub fn revision_changes(
        &self,
        before: &str,
        after: &str,
    ) -> Result<serde_json::Value, EngineError> {
        let previous = self.revision_snapshot(before)?;
        let current = self.revision_snapshot(after)?;
        let incompatible = if previous.root != current.root {
            Some("different_root")
        } else if previous.volume_id.is_none() || current.volume_id.is_none() {
            Some("unknown_volume")
        } else if previous.volume_id != current.volume_id {
            Some("different_volume")
        } else if previous.settings != current.settings {
            Some("different_settings")
        } else if previous.captured_at_unix_ms > current.captured_at_unix_ms {
            Some("out_of_order")
        } else if !previous.coverage.complete || !current.coverage.complete {
            Some("incomplete_coverage")
        } else {
            None
        };
        if let Some(reason) = incompatible {
            return Ok(
                serde_json::json!({"incompatible":reason,"added":0,"removed":0,"size_changed":0,"complete":true,"summary_is_partial":false}),
            );
        }
        let report = self.compare_revisions(before, after, 0)?;
        Ok(serde_json::json!({
            "incompatible": null,
            "added": report.summary.right_only,
            "removed": report.summary.left_only,
            "size_changed": report.rows.iter().filter(|row| row.left_bytes.is_some() && row.right_bytes.is_some() && row.left_bytes != row.right_bytes).count(),
            "complete": report.truncated.is_none(),
            "summary_is_partial": report.truncated.is_some(),
            "truncation_reason": report.truncated,
        }))
    }
}
