use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, DiskNode, QueryReadBudget};
use diskgraph_store::SqliteSnapshotStore;
use std::path::{Component, Path};

impl Engine {
    /// 在原 reader 与累计账本中按相对组件查找历史节点。
    /// 参数：reader/snapshot 为已授权历史，relative 为相对路径，ledger 为同请求额度。
    /// 返回：可选节点；非法/有损路径拒绝，每个实际根/组件解码均计费。
    pub(super) fn history_node_at(
        reader: &SqliteSnapshotStore,
        snapshot: &str,
        relative: &Path,
        ledger: &mut QueryReadBudget,
    ) -> Result<Option<DiskNode>, EngineError> {
        let Some(mut current) = reader.root_node_with_budget(snapshot, ledger)? else {
            return Ok(None);
        };
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(BusinessError::InvalidArgument.into());
            };
            let name = name.to_str().ok_or(BusinessError::InvalidArgument)?;
            current = match reader.child_named_with_budget(snapshot, current.id, name, ledger)? {
                Some(node) => node,
                None => return Ok(None),
            };
        }
        Ok(Some(current))
    }
}
