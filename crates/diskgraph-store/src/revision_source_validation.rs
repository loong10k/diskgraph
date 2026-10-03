//! 历史迁移与新发布共用的版本来源闭包；来源：原生 Rust EV-05 / D28。
use crate::Result;
use rusqlite::Connection;

/// 检查所选运行、原始来源和 active 关系端点属于同一完整选择。
/// 参数：tx 为图库事务，revision 为正在迁移或封存的版本。
/// 返回：缺失、非法 role 或跨快照来源为 false；SQL 失败传播。
pub(crate) fn confirmed(tx: &Connection, revision: &str) -> Result<bool> {
    let incomplete: bool = tx.query_row(
        "SELECT
         EXISTS(SELECT 1 FROM revision_runs rr
             JOIN graph_revisions rev ON rev.revision_id=rr.revision_id
             LEFT JOIN collector_runs cr ON cr.run_id=rr.run_id
             WHERE rr.revision_id=?1 AND (cr.run_id IS NULL OR cr.snapshot_id!=rev.snapshot_id
                 OR rr.role NOT IN ('active','dependency_only')
                 OR CASE WHEN json_valid(cr.run_json) THEN
                     json_extract(cr.run_json,'$.run_id') IS NOT cr.run_id OR
                     json_extract(cr.run_json,'$.snapshot_id') IS NOT rev.snapshot_id
                     ELSE 1 END))
         OR EXISTS(SELECT 1 FROM revision_runs rr
             JOIN graph_revisions rev ON rev.revision_id=rr.revision_id
             JOIN entity_run_memberships m ON m.run_id=rr.run_id AND m.snapshot_id=rev.snapshot_id
             JOIN entities e ON e.snapshot_id=m.snapshot_id AND e.entity_id=m.entity_id
             WHERE rr.revision_id=?1 AND NOT EXISTS(
                 SELECT 1 FROM revision_runs upstream JOIN collector_runs source ON source.run_id=upstream.run_id
                 WHERE upstream.revision_id=rr.revision_id AND source.snapshot_id=m.snapshot_id
                     AND upstream.role IN ('active','dependency_only')
                     AND upstream.run_id=json_extract(e.entity_json,'$.source_run_id')))
         OR EXISTS(SELECT 1 FROM revision_runs rr
             JOIN graph_revisions rev ON rev.revision_id=rr.revision_id
             JOIN relation_run_memberships m ON m.run_id=rr.run_id AND m.snapshot_id=rev.snapshot_id
             JOIN relations edge ON edge.snapshot_id=m.snapshot_id AND edge.edge_id=m.edge_id
             WHERE rr.revision_id=?1 AND rr.role='active' AND (
                 NOT EXISTS(SELECT 1 FROM entity_run_memberships em
                     JOIN revision_runs er ON er.run_id=em.run_id AND er.revision_id=rr.revision_id
                     JOIN collector_runs ec ON ec.run_id=em.run_id AND ec.snapshot_id=em.snapshot_id
                     WHERE em.snapshot_id=m.snapshot_id AND em.entity_id=edge.source_entity_id
                         AND er.role IN ('active','dependency_only'))
                 OR NOT EXISTS(SELECT 1 FROM entity_run_memberships em
                     JOIN revision_runs er ON er.run_id=em.run_id AND er.revision_id=rr.revision_id
                     JOIN collector_runs ec ON ec.run_id=em.run_id AND ec.snapshot_id=em.snapshot_id
                     WHERE em.snapshot_id=m.snapshot_id AND em.entity_id=edge.target_entity_id
                         AND er.role IN ('active','dependency_only'))))",
        [revision], |row| row.get(0))?;
    Ok(!incomplete)
}
