use crate::node_codec::as_i64;
use crate::node_row::NodeRow;
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{DiskNode, QueryReadBudget, ResourceLocator};
use rusqlite::params;

impl SqliteSnapshotStore {
    /// 按精确定位查询一行，并在分配与解码前计入共享账本。
    /// 参数：snapshot/locator 为固定资源，budget 为双侧历史共用的额度和期限。
    /// 返回：节点或 None；借用字段过大时拒绝，不加载其他节点。
    pub fn node_by_locator_with_budget(
        &self,
        snapshot: &str,
        locator: &ResourceLocator,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<DiskNode>> {
        let locator = serde_json::to_string(locator)?;
        // 根不在路径表达式的部分索引内，先走父索引；非根走既有路径索引。
        // 最后的原始 locator 相等条件保留类型与无损定位语义。
        let root = self.one_node_with_budget("SELECT id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error FROM nodes WHERE snapshot_id=?1 AND parent_id IS NULL AND locator_key=?2 LIMIT 1", params![snapshot,locator], budget)?;
        if root.is_some() {
            return Ok(root);
        }
        self.one_node_with_budget("SELECT id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error FROM nodes WHERE snapshot_id=?1 AND parent_id IS NOT NULL AND json_extract(locator_key,'$.value')=json_extract(?2,'$.value') AND locator_key=?2 LIMIT 1", params![snapshot,locator], budget)
    }

    /// 读取根之前对借用列与实际解码数量准入。
    /// 参数：snapshot 为固定快照，budget 为准备及数据阶段共享账本。
    /// 返回：根或 None；预算拒绝不拥有其字符串，真实错误保留。
    pub fn root_node_with_budget(
        &self,
        snapshot: &str,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<DiskNode>> {
        self.one_node_with_budget("SELECT id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error FROM nodes WHERE snapshot_id=?1 AND parent_id IS NULL LIMIT 1", params![snapshot], budget)
    }

    /// 在共享账本中逐组件查找时读取一个精确子节点。
    /// 参数：snapshot/parent/name 为固定身份，budget 为请求实际解码账本。
    /// 返回：可选节点；不因新查找重置期限或节点/原始字段额度。
    pub fn child_named_with_budget(
        &self,
        snapshot: &str,
        parent: u64,
        name: &str,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<DiskNode>> {
        self.one_node_with_budget("SELECT id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error FROM nodes WHERE snapshot_id=?1 AND parent_id=?2 AND name=?3 LIMIT 1", params![snapshot,as_i64(parent)?,name], budget)
    }

    fn one_node_with_budget(
        &self,
        sql: &str,
        parameters: impl rusqlite::Params,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<DiskNode>> {
        self.one_node_with_admission(sql, parameters, &mut |raw, entries, _| {
            if budget.admit(
                usize::try_from(entries).map_err(|_| StoreError::BudgetExceeded)?,
                0,
                usize::try_from(raw).map_err(|_| StoreError::BudgetExceeded)?,
            ) {
                Ok(())
            } else {
                Err(StoreError::BudgetExceeded)
            }
        })
    }

    fn one_node_with_admission(
        &self,
        sql: &str,
        parameters: impl rusqlite::Params,
        admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
    ) -> Result<Option<DiskNode>> {
        admit(0, 0, 0)?;
        let mut statement = self.connection.prepare(sql)?;
        let mut rows = statement.query(parameters)?;
        let row = rows.next()?;
        admit(0, 0, 0)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let json_column = if row.get_ref(6)? == rusqlite::types::ValueRef::Null {
            5
        } else {
            2
        };
        let json = match row.get_ref(json_column)? {
            rusqlite::types::ValueRef::Text(bytes) | rusqlite::types::ValueRef::Blob(bytes) => {
                bytes.len()
            }
            _ => 0,
        };
        crate::metadata_read_cost::charge(
            admit,
            NodeRow::raw_bytes(row)?,
            json,
            std::mem::size_of::<NodeRow>() + std::mem::size_of::<DiskNode>(),
            1,
        )?;
        let node = NodeRow::from_row(row)?.into_node()?;
        admit(0, 0, 0)?;
        Ok(Some(node))
    }

    /// 对精确节点的所有借用列先准入，再拥有字符串/解码旧 JSON。
    /// 参数：snapshot/node_id 固定节点，budget 为同请求累计原始字段/节点账本。
    /// 返回：节点或 None；超限返回 BudgetExceeded，真实列/JSON错误保留。
    pub fn node_with_budget(
        &self,
        snapshot: &str,
        node_id: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<DiskNode>> {
        self.one_node_with_budget(NODE_SQL, params![snapshot, as_i64(node_id)?], budget)
    }

    /// 参数：固定快照/node及原会话回调；返回：解码前raw/容器放大准入的完整节点。
    pub fn node_with_admission(
        &self,
        snapshot: &str,
        node_id: u64,
        admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
    ) -> Result<Option<DiskNode>> {
        self.one_node_with_admission(NODE_SQL, params![snapshot, as_i64(node_id)?], admit)
    }
}

const NODE_SQL: &str = "SELECT id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error FROM nodes WHERE snapshot_id=?1 AND id=?2 LIMIT 1";
