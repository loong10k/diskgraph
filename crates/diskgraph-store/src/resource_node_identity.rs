//! 占用及保护关系的资源定位确认；来源：原生 Rust EV-05 / D28。
use crate::Result;
use diskgraph_core::{Entity, EntityKind};
use rusqlite::{Connection, params};

/// 确认资源身份能够映射到本快照实际节点。
/// 参数：connection 为当前事务，snapshot 为实际快照，entity 为资源观察。
/// 返回：身份缺失或非法为 false；数据库读取失败仍传播。
pub(crate) fn confirmed_node(
    connection: &Connection,
    snapshot: &str,
    entity: &Entity,
) -> Result<bool> {
    if entity.kind != EntityKind::Resource {
        return Ok(false);
    }
    let Ok(identity) = serde_json::from_str::<serde_json::Value>(&entity.identity) else {
        return Ok(false);
    };
    let Some(node) = identity
        .get("node_id")
        .and_then(serde_json::Value::as_u64)
        .filter(|node| *node > 0)
        .and_then(|node| i64::try_from(node).ok())
    else {
        return Ok(false);
    };
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM nodes WHERE snapshot_id=?1 AND id=?2)",
        params![snapshot, node],
        |row| row.get(0),
    )?)
}
