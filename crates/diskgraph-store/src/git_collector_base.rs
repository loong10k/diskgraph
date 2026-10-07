//! Git 发布的实际 namespace 基线资格；来源：原生 Rust EC-02 / D34 历史归属。
//! 所有检查使用调用方已持有的同一个 IMMEDIATE 图库事务，不另设状态 owner 或最新指针表。

use crate::{Result, StoreError};
use diskgraph_core::GitEvidenceJobInput;
use rusqlite::{Connection, OptionalExtension, params};

/// 参数：当前事务、固定输入及原快照 root_key；返回：实际 owner 最新且没有未绑定更新时成功。
/// 复用既有历史排序；不能把其他 owner 的显示根指针当成本 scope 的版本。
pub(crate) fn require_current(
    connection: &Connection,
    input: &GitEvidenceJobInput,
    root: &str,
) -> Result<()> {
    require_latest(connection, input, input.base_revision_id())?;
    // 同 owner 或 unowned 的根指针已前进时保留旧可信扫描冲突；缺指针视为损坏基线。
    let incompatible_pointer: bool = connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM latest_revision WHERE root_key=?1)
         OR EXISTS(SELECT 1 FROM latest_revision l LEFT JOIN revision_authorized_ownership o
             ON o.revision_id=l.revision_id WHERE l.root_key=?1 AND l.revision_id!=?2
             AND (o.revision_id IS NULL OR (o.server_id=?3 AND o.scope_id=?4)))",
        params![
            root,
            input.base_revision_id(),
            input.server_id().as_str(),
            input.scope_id().as_str()
        ],
        |row| row.get(0),
    )?;
    // 未绑定扫描不能被后来另一个 owner 的 pointer 覆盖而抹去：按同一历史排序比较基线。
    let unbound_update: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM graph_revisions r JOIN snapshots s ON s.id=r.snapshot_id
         JOIN graph_revisions base ON base.revision_id=?2
         LEFT JOIN revision_authorized_ownership o ON o.revision_id=r.revision_id
         WHERE s.root_key=?1 AND o.revision_id IS NULL AND
         (r.published_at_unix_ms>base.published_at_unix_ms OR
          (r.published_at_unix_ms=base.published_at_unix_ms AND r.revision_id>base.revision_id)))",
        params![root, input.base_revision_id()],
        |row| row.get(0),
    )?;
    if incompatible_pointer || unbound_update {
        return Err(StoreError::Conflict(
            "stale Git root; refresh from an owned scan".into(),
        ));
    }
    Ok(())
}

/// 参数：同一图库事务、实际 owner 及期望版本；返回：严格匹配已存在历史排序的最新版本。
/// 提交前也使用本检查，发布时间或 ID 排序倒退不能产生已完成但不是实际最新的回执。
pub(crate) fn require_latest(
    connection: &Connection,
    input: &GitEvidenceJobInput,
    revision: &str,
) -> Result<()> {
    let matches: Option<bool> = connection
        .query_row(
            "SELECT r.revision_id=?3 FROM graph_revisions r JOIN revision_authorized_ownership o
         ON o.revision_id=r.revision_id WHERE o.server_id=?1 AND o.scope_id=?2
         ORDER BY r.published_at_unix_ms DESC,r.revision_id DESC LIMIT 1",
            params![
                input.server_id().as_str(),
                input.scope_id().as_str(),
                revision
            ],
            |row| row.get(0),
        )
        .optional()?;
    if matches != Some(true) {
        return Err(StoreError::Conflict(
            "stale Git base; refresh from latest owned revision".into(),
        ));
    }
    Ok(())
}
