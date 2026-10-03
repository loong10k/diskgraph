//! 显示别名发布时逐一核对完整节点和原始定位，禁止仅以数量代替身份。

use crate::{Result, StoreError};
use diskgraph_core::{
    DiskGraph, DiskNode, LocatorEncoding, LocatorKind, QualifiedLocator, ResourceLocator,
};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// 逐项校验暂存节点与完成图完全一致，并验证显示别名的原始身份唯一性。
/// 参数：tx 为当前发布事务连接，job 为暂存命名空间，graph 为已完成观测图。
/// 返回：完整一致时成功；缺失、冲突或损坏记录返回错误。
pub(crate) fn validate_aliases(tx: &Connection, job: &str, graph: &DiskGraph) -> Result<()> {
    let expected = graph
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut identities = HashSet::new();
    let mut stmt = tx.prepare("SELECT node_json,native_locator_kind,native_locator_encoding,native_locator_raw,self_modified_unix_seconds FROM scan_staging WHERE job_id=?1")?;
    let mut rows = stmt.query([job])?;
    while let Some(row) = rows.next()? {
        let node: DiskNode = serde_json::from_str(&row.get::<_, String>(0)?)?;
        if expected
            .get(&node.id)
            .is_none_or(|expected| *expected != &node)
            || !seen.insert(node.id)
        {
            return Err(invalid("staged node identity differs from completed graph"));
        }
        let kind: String = row.get(1)?;
        let encoding: String = row.get(2)?;
        let raw: Vec<u8> = row.get(3)?;
        let modified: Option<i64> = row.get(4)?;
        if !identities.insert((kind.clone(), encoding.clone(), raw.clone())) {
            return Err(invalid(
                "display aliases require distinct raw locator identities",
            ));
        }
        let kind = match kind.as_str() {
            "native_path" => LocatorKind::NativePath,
            "document_uri" => LocatorKind::DocumentUri,
            _ => return Err(invalid("unknown staged locator kind")),
        };
        let encoding =
            LocatorEncoding::parse(&encoding).map_err(|error| invalid(&error.to_string()))?;
        let display = match &node.locator {
            ResourceLocator::NativePath(display) | ResourceLocator::DocumentUri(display) => {
                display.clone()
            }
        };
        let locator = QualifiedLocator::from_parts(kind, encoding, raw, display)
            .map_err(|error| invalid(&error.to_string()))?;
        crate::staging_node_encoding::StagingNodeEncoding::encode(&node, Some(&locator), modified)?;
    }
    if seen.len() != expected.len() {
        return Err(invalid("staged node set does not match completed graph"));
    }
    Ok(())
}
fn invalid(reason: &str) -> StoreError {
    StoreError::InvalidGraph(reason.into())
}
