//! 采集批次邻接索引：历史批次不参与当前页扫描。
use crate::Result;
use rusqlite::Connection;

/// 创建规范化成员的邻接投影，供每个有效批次按 keyset 定位。
/// 参数：tx 为创建成员表的迁移事务。
/// 返回：投影、索引及同步触发器全部建立，失败由外层回滚。
pub(crate) fn create(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE relation_membership_adjacency (
            snapshot_id TEXT NOT NULL, run_id TEXT NOT NULL, edge_id TEXT NOT NULL,
            source_entity_id TEXT NOT NULL, target_entity_id TEXT NOT NULL, relation TEXT NOT NULL,
            PRIMARY KEY(snapshot_id,run_id,edge_id),
            FOREIGN KEY(snapshot_id,run_id,edge_id) REFERENCES relation_run_memberships(snapshot_id,run_id,edge_id) ON DELETE CASCADE);
         CREATE INDEX membership_source_page ON relation_membership_adjacency(snapshot_id,run_id,source_entity_id,edge_id);
         CREATE INDEX membership_target_page ON relation_membership_adjacency(snapshot_id,run_id,target_entity_id,edge_id);
         CREATE TRIGGER relation_membership_index_insert AFTER INSERT ON relation_run_memberships
         BEGIN INSERT INTO relation_membership_adjacency
            SELECT NEW.snapshot_id,NEW.run_id,NEW.edge_id,source_entity_id,target_entity_id,relation
            FROM relations WHERE snapshot_id=NEW.snapshot_id AND edge_id=NEW.edge_id; END;
         CREATE TRIGGER relation_membership_retarget AFTER UPDATE OF source_entity_id,target_entity_id,relation ON relations
         BEGIN UPDATE relation_membership_adjacency SET source_entity_id=NEW.source_entity_id,
             target_entity_id=NEW.target_entity_id,relation=NEW.relation
             WHERE snapshot_id=NEW.snapshot_id AND edge_id=NEW.edge_id; END;",
    )?;
    Ok(())
}
