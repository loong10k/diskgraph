use crate::node_codec::kind_from_name;
use crate::{Result, StoreError};
use diskgraph_core::{NodeKind, QueryReadBudget};
use serde::Deserialize;

/// 导航与地图真正使用的节点投影，不拥有定位、回收提示或文件身份。
/// 来源：DiskGraph 原生 Rust Q-08 / 13.6；无 Java 对应对象。
#[derive(Debug, Deserialize)]
pub struct NavigationNode {
    pub id: u64,
    pub name: String,
    pub kind: NodeKind,
    pub subtree_bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub category_hint: Option<String>,
    pub read_error: bool,
    pub size_known: bool,
}

impl NavigationNode {
    /// 计量固定投影列的借用长度，旧 JSON 尚未拥有或解析。
    /// 参数：row 为九列导航投影。返回：原始字节成本或真实列读取错误。
    fn raw_bytes(row: &rusqlite::Row<'_>) -> Result<usize> {
        let mut bytes = 0usize;
        for column in 0..9 {
            let length = match row.get_ref(column)? {
                rusqlite::types::ValueRef::Null => 0,
                rusqlite::types::ValueRef::Integer(_) | rusqlite::types::ValueRef::Real(_) => 8,
                rusqlite::types::ValueRef::Text(value) | rusqlite::types::ValueRef::Blob(value) => {
                    value.len()
                }
            };
            bytes = bytes
                .checked_add(length)
                .ok_or(StoreError::BudgetExceeded)?;
        }
        Ok(bytes)
    }

    /// 先准入实际节点及原始字段，再按既有结构化或旧 JSON 语义解码必要字段。
    /// 参数：row 为固定投影，budget 贯穿整次导航或整帧。返回：投影或预算/真实格式错误。
    pub(crate) fn from_row(row: &rusqlite::Row<'_>, budget: &mut QueryReadBudget) -> Result<Self> {
        if !budget.admit(1, 0, Self::raw_bytes(row)?) {
            return Err(StoreError::BudgetExceeded);
        }
        let id: i64 = row.get(0)?;
        let kind: Option<String> = row.get(2)?;
        let Some(kind) = kind else {
            // 旧行只有 JSON 是字段事实来源；先对完整输入准入，serde 只拥有必要字段。
            let json: String = row.get(8)?;
            let node: Self = serde_json::from_str(&json)?;
            if node.id != id as u64 {
                return Err(StoreError::InvalidGraph(
                    "node payload ID differs from its row".into(),
                ));
            }
            return Ok(node);
        };
        Ok(Self {
            id: id as u64,
            name: row.get(1)?,
            kind: kind_from_name(&kind)?,
            subtree_bytes: row.get::<_, i64>(3)? as u64,
            files: row.get::<_, Option<i64>>(4)?.unwrap_or(0) as u64,
            directories: row.get::<_, Option<i64>>(5)?.unwrap_or(0) as u64,
            category_hint: row.get(6)?,
            read_error: row.get::<_, Option<i64>>(7)?.unwrap_or(0) != 0,
            size_known: true,
        })
    }
}
