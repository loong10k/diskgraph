//! 暂存节点编码及 Engine/Store 共享的实际字段成本。

use crate::{Result, StoreError};
use diskgraph_core::{DiskNode, LocatorKind, QualifiedLocator, ResourceLocator};

/// 单个暂存节点实际写入的 JSON 和搜索字段，避免计量与写入采用不同编码。
/// 来源：DiskGraph 原生 Rust D30；无 Java 对应对象。
pub(crate) struct StagingNodeEncoding {
    pub(crate) json: String,
    pub(crate) name_fold: String,
    pub(crate) path_fold: String,
    pub(crate) cost: u64,
}

impl StagingNodeEncoding {
    /// 校验定位并编码节点 JSON、搜索字段与 checked 字节成本。
    /// 参数：node 为节点，locator 为可选无损定位，self_modified 为可选自身修改秒数。
    /// 返回：可直接暂存的编码结果；身份冲突、序列化或溢出返回错误。
    pub(crate) fn encode(
        node: &DiskNode,
        locator: Option<&QualifiedLocator>,
        self_modified: Option<i64>,
    ) -> Result<Self> {
        let path = match &node.locator {
            ResourceLocator::NativePath(path) | ResourceLocator::DocumentUri(path) => path,
        };
        if let Some(locator) = locator {
            let matching_kind = matches!(
                (&node.locator, locator.kind()),
                (ResourceLocator::NativePath(_), LocatorKind::NativePath)
                    | (ResourceLocator::DocumentUri(_), LocatorKind::DocumentUri)
            );
            if !matching_kind || path != locator.display() {
                return Err(StoreError::InvalidGraph(
                    "qualified locator differs from node display or kind".into(),
                ));
            }
            // 公开字段仍须再次验证；编码合法不等于当前宿主可寻址，native reader 会单独检查宿主。
            locator.validate().map_err(|error| {
                StoreError::InvalidGraph(format!("invalid staged locator: {error}"))
            })?;
        } else if self_modified.is_some() {
            return Err(StoreError::InvalidGraph(
                "self modification time requires an observed qualified locator".into(),
            ));
        }
        let json = serde_json::to_string(node)?;
        let name_fold = node.name.to_lowercase();
        let path_fold = path.to_lowercase();
        let mut cost = 0u64;
        for size in [json.len(), name_fold.len(), path_fold.len()] {
            cost = add_size(cost, size)?;
        }
        if let Some(locator) = locator {
            for size in [
                locator.raw_bytes().len(),
                kind_name(locator.kind()).len(),
                locator.encoding().wire_name().len(),
            ] {
                cost = add_size(cost, size)?;
            }
        }
        if self_modified.is_some() {
            cost = cost.checked_add(8).ok_or(StoreError::IntegerOverflow)?;
        }
        Ok(Self {
            json,
            name_fold,
            path_fold,
            cost,
        })
    }
}

/// 返回暂存字段的实际编码字节数，供扫描累计预算和批次写入使用同一实现。
/// 参数：节点、可选已验证定位和可选自身修改时间；旧节点使用 None/None。
/// 返回：JSON、Unicode 折叠搜索字段、原始定位、标签及时间的 checked 合计；不估算 SQLite 页开销。
pub fn staging_node_encoded_cost(
    node: &DiskNode,
    locator: Option<&QualifiedLocator>,
    self_modified: Option<i64>,
) -> Result<u64> {
    Ok(StagingNodeEncoding::encode(node, locator, self_modified)?.cost)
}

/// 将定位类型转换为稳定的数据库标签。
/// 参数：kind 为已识别的资源定位类型。
/// 返回：对应类型的静态 snake_case 标签。
pub(crate) fn kind_name(kind: LocatorKind) -> &'static str {
    match kind {
        LocatorKind::NativePath => "native_path",
        LocatorKind::DocumentUri => "document_uri",
    }
}

fn add_size(total: u64, size: usize) -> Result<u64> {
    total
        .checked_add(u64::try_from(size).map_err(|_| StoreError::IntegerOverflow)?)
        .ok_or(StoreError::IntegerOverflow)
}
