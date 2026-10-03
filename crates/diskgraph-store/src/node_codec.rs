use crate::{Result, StoreError};
use diskgraph_core::DiskNode;
use serde_json::to_string;

use crate::node_row::NodeRow;

/// NodeKind's stable snake-case wire name (must match serde's rendering).
/// 编码或解析既有稳定 wire 标签。
/// 参数：kind：节点或任务类型。
/// 返回：directory/file/symlink/other 稳定标签。
pub(crate) fn kind_name(kind: diskgraph_core::NodeKind) -> &'static str {
    match kind {
        diskgraph_core::NodeKind::Directory => "directory",
        diskgraph_core::NodeKind::File => "file",
        diskgraph_core::NodeKind::Symlink => "symlink",
        diskgraph_core::NodeKind::Other => "other",
    }
}

/// Parses the wire name written by kind_name; unknown values are corrupt rows.
/// 编码或解析既有稳定 wire 标签。
/// 参数：name：精确名称或 wire 标签。
/// 返回：对应 NodeKind，未知标签返回 InvalidGraph。
pub(crate) fn kind_from_name(name: &str) -> Result<diskgraph_core::NodeKind> {
    Ok(match name {
        "directory" => diskgraph_core::NodeKind::Directory,
        "file" => diskgraph_core::NodeKind::File,
        "symlink" => diskgraph_core::NodeKind::Symlink,
        "other" => diskgraph_core::NodeKind::Other,
        _ => {
            return Err(StoreError::InvalidGraph(format!(
                "unknown node kind: {name}"
            )));
        }
    })
}

/// 无损转换 SQLite 整数，超界拒绝。
/// 参数：value：待编码/解析字段。
/// 返回：无损 SQLite 整数，超界返回 IntegerOverflow。
pub(crate) fn as_i64(value: u64) -> Result<i64> {
    value.try_into().map_err(|_| StoreError::IntegerOverflow)
}

/// The archived JSON payload for one node row.
///
/// Every measured node also has its fields in the structured columns, and
/// the read path reconstructs it from those — it never parses the payload
/// for such a row. Writing it anyway cost half a kilobyte per node, so a
/// four-million-node index carried two and a half gigabytes of a copy of
/// data already in the row. Only a node whose size is unknown still has the
/// payload as its record, and that is the one case that gets one.
/// 保留未知大小 JSON 和已测量结构化行的还原语义。
/// 参数：node：完整观测节点。
/// 返回：未知大小节点完整 JSON；已测量节点空字符串，改由结构化列保存。
pub(crate) fn payload_for(node: &DiskNode) -> Result<String> {
    if node.size_known {
        Ok(String::new())
    } else {
        Ok(to_string(node)?)
    }
}

/// The `kind` column doubles as the marker for a fully measured row: the read
/// path reconstructs a node from the columns when it is set, and falls back to
/// the archived payload when it is not.
///
/// A node whose size could not be measured therefore leaves it empty. Filling
/// it anyway would read the node back as measured — "unknown" silently
/// becoming "zero bytes", which is the one thing a size report must never
/// do to a directory it could not open.
/// 保留未知大小 JSON 和已测量结构化行的还原语义。
/// 参数：node：完整观测节点。
/// 返回：已测量节点的类型标签；未知大小为 None，不能伪装成零字节。
pub(crate) fn measured_kind(node: &DiskNode) -> Option<&'static str> {
    if node.size_known {
        Some(kind_name(node.kind))
    } else {
        None
    }
}

/// 页节点才构造/解析负载；多出来的一行只确认存在，不复制它的 JSON/定位字符串。
/// 仅解码页内节点，额外 lookahead 只确认存在。
/// 参数：statement：已准备的有界节点语句；parameters：参数化 SQL 绑定值；limit：最大页条数。
/// 返回：当前页节点及 lookahead 是否存在。
pub(crate) fn read_node_page(
    statement: &mut rusqlite::Statement<'_>,
    parameters: impl rusqlite::Params,
    limit: u64,
) -> Result<(Vec<DiskNode>, bool)> {
    let mut rows = statement.query(parameters)?;
    let mut items = Vec::new();
    while (items.len() as u64) < limit {
        let Some(row) = rows.next()? else {
            break;
        };
        items.push(NodeRow::from_row(row)?.into_node()?);
    }
    let more = rows.next()?.is_some();
    Ok((items, more))
}
