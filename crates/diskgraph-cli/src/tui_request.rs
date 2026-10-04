use diskgraph_core::{Authorizer, PrincipalId, QueryBudget, QueryReadBudget};
use diskgraph_engine::{Engine, EngineError};
use diskgraph_store::SqliteSnapshotStore;
use std::time::Instant;

use crate::tui::{Layer, layer_from_navigation_nodes};

pub(crate) const TUI_NODE_LIMIT: usize = 2048;
pub(crate) const TUI_PREPARATION_BYTES: usize = 256 * 1024;
pub(crate) const TUI_DISPLAY_BYTES: usize = 256 * 1024;

/// 终端会话的不可变请求身份；每帧和导航读取仍检查实时授权。
/// 来源：DiskGraph 原生 Rust Q-08 / 13.6；无 Java 对应对象。
pub struct TuiRequest<'a> {
    pub engine: &'a Engine,
    pub revision: &'a str,
    pub principal: &'a PrincipalId,
    pub authorizer: &'a dyn Authorizer,
}

impl TuiRequest<'_> {
    /// 为导航或整帧建立必要字段读取账本，不重新计时或将展示字节冒充原始输入。
    /// 参数：deadline 来自已授权 reader。返回：固定正数预算的请求局部账本。
    pub(crate) fn read_budget(deadline: Instant) -> QueryReadBudget {
        QueryReadBudget::new(
            QueryBudget {
                max_nodes: TUI_NODE_LIMIT,
                max_response_bytes: TUI_PREPARATION_BYTES,
                ..QueryBudget::default()
            },
            deadline,
        )
        .expect("TUI has fixed nonzero read limits")
    }

    /// 在同一账本读取父节点和一个子页，不读取导航之外的定位或回收提示。
    /// 参数：reader/snapshot 由真实 revision 授权产生；parent/offset/limit 定义页。
    /// 返回：兼容导航层或真实错误；不把缺少的数据当成完整父块继续缩放。
    pub(crate) fn read_layer(
        reader: &SqliteSnapshotStore,
        snapshot: &str,
        parent: u64,
        offset: u64,
        limit: usize,
        budget: &mut QueryReadBudget,
    ) -> Result<Layer, EngineError> {
        let node = reader
            .navigation_node_with_budget(snapshot, parent, budget)?
            .ok_or(EngineError::Business(
                diskgraph_core::BusinessError::NotFound,
            ))?;
        let (children, more) = reader.navigation_children_with_budget(
            snapshot,
            parent,
            offset,
            limit as u64,
            budget,
        )?;
        if !budget.check() {
            return Err(diskgraph_store::StoreError::BudgetExceeded.into());
        }
        Ok(layer_from_navigation_nodes(node, children, offset, more))
    }
}
