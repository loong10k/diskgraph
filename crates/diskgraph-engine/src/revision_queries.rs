//! 共享 Engine 的 revision_queries 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError};
use diskgraph_core::BusinessError;

impl Engine {
    /// 可信窄读 revision 的快照元信息。
    /// 参数：revision_id 为已由调用方授权的历史标识。
    /// 返回：快照信息或读取失败。
    /// 读取 revision 的快照元信息，避免加载所有节点。
    pub fn revision_snapshot(
        &self,
        revision_id: &str,
    ) -> Result<diskgraph_core::DiskSnapshot, EngineError> {
        let reader = self.revision_reader()?;
        Ok(reader.snapshot(&reader.revision(revision_id)?.snapshot_id)?)
    }
}

impl Engine {
    /// 可信窄读历史中一个节点。
    /// 参数：revision_id/node_id 须由调用方先授权。
    /// 返回：节点或 not_found/存储失败。
    /// 读取指定 revision 中单个节点。
    pub fn revision_node(
        &self,
        revision_id: &str,
        node_id: u64,
    ) -> Result<diskgraph_core::DiskNode, EngineError> {
        let reader = self.revision_reader()?;
        reader
            .node(&reader.revision(revision_id)?.snapshot_id, node_id)?
            .ok_or(EngineError::Business(BusinessError::NotFound))
    }
}

impl Engine {
    /// 可信窄读一页已知大小子节点。
    /// 参数：revision/parent、minimum、offset/limit 为已授权查询。
    /// 返回：节点、next offset 和未知数；页上限100。
    /// 读取目录的一页，保留 offset 和未知大小计数兼容字段。
    pub fn revision_children_page(
        &self,
        revision_id: &str,
        parent_id: u64,
        minimum: Option<u64>,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::DiskNode>, Option<u64>, u64), EngineError> {
        let reader = self.revision_reader()?;
        Ok(reader.children_page(
            &reader.revision(revision_id)?.snapshot_id,
            parent_id,
            minimum,
            offset,
            limit.min(100),
        )?)
    }
}

impl Engine {
    /// 可信窄读一页未知大小子节点。
    /// 参数：revision/parent、offset/limit 为已授权查询。
    /// 返回：节点和 next offset；页上限100。
    /// 读取未知大小子节点的一页；调用方必须先完成 revision 授权。
    pub fn revision_unknown_children_page(
        &self,
        revision_id: &str,
        parent_id: u64,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::DiskNode>, Option<u64>), EngineError> {
        let reader = self.revision_reader()?;
        Ok(reader.unknown_children_page(
            &reader.revision(revision_id)?.snapshot_id,
            parent_id,
            offset,
            limit.min(100),
        )?)
    }
}

impl Engine {
    /// 可信窄读最大子节点并检查后续存在。
    /// 参数：revision/parent 与 limit 为已授权查询。
    /// 返回：最多100个节点与更多标志。
    /// 读取最大子节点，不为少量 CLI 结果解码完整 revision。
    pub fn revision_top(
        &self,
        revision_id: &str,
        parent_id: u64,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::DiskNode>, bool), EngineError> {
        let reader = self.revision_reader()?;
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        let limit = limit.min(100);
        let mut items = reader.top(&snapshot_id, parent_id, limit.saturating_add(1))?;
        let more = items.len() as u64 > limit;
        items.truncate(limit as usize);
        Ok((items, more))
    }
}

impl Engine {
    /// 可信窄读已发布历史的根节点。
    /// 参数：revision_id 须由调用方授权。
    /// 返回：一个根节点或 not_found/存储失败。
    /// The root node of a published revision - the entry a du-style summary
    /// reads its total from. One row, no graph materialization.
    pub fn revision_root_node(
        &self,
        revision_id: &str,
    ) -> Result<diskgraph_core::DiskNode, EngineError> {
        let graph = self.revision_reader()?;
        let record = graph.revision(revision_id)?;
        graph
            .root_node(&record.snapshot_id)?
            .ok_or(EngineError::Business(BusinessError::NotFound))
    }
}

impl Engine {
    /// 可信按相对路径组件逐层读取单节点。
    /// 参数：revision_id 与 relative 为已授权定位。
    /// 返回：可选节点；父跳转非法，不推断重命名。
    /// One node in a published revision, addressed by its path under the
    /// root.
    ///
    /// Walks the path one level at a time, so the cost is one indexed lookup
    /// per segment rather than a scan. Loading the revision to answer "how
    /// did this directory change" was the only way before, and on a
    /// four-million-node index that meant materializing four million nodes to
    /// read two of them.
    pub fn revision_node_at(
        &self,
        revision_id: &str,
        relative: &std::path::Path,
    ) -> Result<Option<diskgraph_core::DiskNode>, EngineError> {
        use diskgraph_core::ResourceLocator;
        let graph = self.revision_reader()?;
        let record = graph.revision(revision_id)?;
        let mut current = match graph.root_node(&record.snapshot_id)? {
            Some(root) => root,
            None => return Ok(None),
        };
        for segment in relative.components() {
            let name = segment.as_os_str().to_string_lossy().into_owned();
            if name == "." || name == ".." {
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            }
            current = match graph.child_named(&record.snapshot_id, current.id, &name)? {
                Some(child) => child,
                None => return Ok(None),
            };
        }
        // A path that names the root itself is the root, not a miss.
        debug_assert!(matches!(current.locator, ResourceLocator::NativePath(_)));
        Ok(Some(current))
    }
}

impl Engine {
    /// 可信读取一层目录节点及子节点。
    /// 参数：revision/parent 与 limit 为已授权查询。
    /// 返回：目录与按观察大小排序的子节点。
    /// One directory level of a published revision: the node itself and its
    /// children, ordered by observed size. The interactive surface loads a
    /// level at a time, so a multi-million-node index opens without
    /// materializing the whole graph.
    pub fn revision_layer(
        &self,
        revision_id: &str,
        parent_id: u64,
        limit: usize,
    ) -> Result<(diskgraph_core::DiskNode, Vec<diskgraph_core::DiskNode>), EngineError> {
        let (node, children, _) = self.revision_layer_page(revision_id, parent_id, 0, limit)?;
        Ok((node, children))
    }
}

impl Engine {
    /// 可信读取有 offset 和 lookahead 的目录页。
    /// 参数：revision/parent、offset/limit 为已授权查询。
    /// 返回：目录、子节点及明确更多标志。
    /// A directory page with an explicit next-page indicator for wide folders.
    pub fn revision_layer_page(
        &self,
        revision_id: &str,
        parent_id: u64,
        offset: u64,
        limit: usize,
    ) -> Result<
        (
            diskgraph_core::DiskNode,
            Vec<diskgraph_core::DiskNode>,
            bool,
        ),
        EngineError,
    > {
        let graph = self.revision_reader()?;
        let record = graph.revision(revision_id)?;
        let node = graph
            .node(&record.snapshot_id, parent_id)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let mut children = graph.children(
            &record.snapshot_id,
            parent_id,
            offset,
            (limit as u64).saturating_add(1),
        )?;
        let more = children.len() > limit;
        children.truncate(limit);
        Ok((node, children, more))
    }
}
