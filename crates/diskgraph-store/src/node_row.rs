use crate::Result;
use crate::node_codec::kind_from_name;
use diskgraph_core::{DiskNode, ResourceLocator};
use serde_json::from_str;

/// 结构化节点及旧 JSON 行的解码对象。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// One row of a node query, in whichever of the two shapes it was stored:
/// a measured node with its fields in columns, or a pre-v4 node whose
/// fields live only in the archived payload.
pub(crate) struct NodeRow {
    id: i64,
    parent_id: Option<i64>,
    locator_key: String,
    name: String,
    subtree_bytes: i64,
    node_json: String,
    kind: Option<String>,
    direct_bytes: Option<i64>,
    files: Option<i64>,
    directories: Option<i64>,
    modified_unix_seconds: Option<i64>,
    file_volume_id: Option<String>,
    file_id: Option<i64>,
    category_hint: Option<String>,
    reclaim_hint: Option<String>,
    read_error: Option<i64>,
}

impl<'row> From<&'row rusqlite::Row<'row>> for NodeRow {
    fn from(row: &'row rusqlite::Row<'row>) -> Self {
        Self {
            id: row.get(0).unwrap_or_default(),
            parent_id: row.get(1).unwrap_or_default(),
            locator_key: row.get(2).unwrap_or_default(),
            name: row.get(3).unwrap_or_default(),
            subtree_bytes: row.get(4).unwrap_or_default(),
            node_json: row.get(5).unwrap_or_default(),
            kind: row.get(6).unwrap_or_default(),
            direct_bytes: row.get(7).unwrap_or_default(),
            files: row.get(8).unwrap_or_default(),
            directories: row.get(9).unwrap_or_default(),
            modified_unix_seconds: row.get(10).unwrap_or_default(),
            file_volume_id: row.get(11).unwrap_or_default(),
            file_id: row.get(12).unwrap_or_default(),
            category_hint: row.get(13).unwrap_or_default(),
            reclaim_hint: row.get(14).unwrap_or_default(),
            read_error: row.get(15).unwrap_or_default(),
        }
    }
}

impl NodeRow {
    /// The node this row describes. A row with a structured `kind` is read
    /// from its columns; a pre-v4 row falls back to the payload, which is
    /// where its fields live.
    /// 保留未知大小 JSON 和已测量结构化行的还原语义。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：还原节点；非法定位、类型或 JSON 返回错误。
    pub(crate) fn into_node(self) -> Result<DiskNode> {
        let Some(kind) = self.kind else {
            return Ok(from_str(&self.node_json)?);
        };
        let locator: ResourceLocator = from_str(&self.locator_key)?;
        let file_identity = match (self.file_volume_id, self.file_id) {
            (Some(volume_id), Some(id)) => Some(diskgraph_core::FileIdentity {
                volume_id,
                file_id: id as u64,
            }),
            _ => None,
        };
        Ok(DiskNode {
            id: self.id as u64,
            parent_id: self.parent_id.map(|value| value as u64),
            locator,
            name: self.name,
            kind: kind_from_name(&kind)?,
            subtree_bytes: self.subtree_bytes as u64,
            direct_bytes: self.direct_bytes.unwrap_or(0) as u64,
            files: self.files.unwrap_or(0) as u64,
            directories: self.directories.unwrap_or(0) as u64,
            modified_unix_seconds: self.modified_unix_seconds,
            file_identity,
            category_hint: self.category_hint,
            reclaim_hint: self.reclaim_hint,
            read_error: self.read_error.unwrap_or(0) != 0,
            // Structured rows are only written for fully measured nodes;
            // unknown sizes stay in the payload.
            size_known: true,
        })
    }
}
