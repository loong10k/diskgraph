//! 元数据疑似重复分组。

use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, DiskGraph, ScopeId};

impl Engine {
    /// 可信元数据兼容入口对范围最新图分组疑似重复。
    /// 参数：scope_id 为已由调用方授权的范围。
    /// 返回：大小/身份疑似组，不证明正文相同。
    /// The metadata-only duplicate suspects of a published revision (CT-03,
    /// task 8.3): equal file sizes group as suspects, hard-linked names are
    /// annotated, and nothing here claims content equality.
    pub fn duplicate_suspects(
        &self,
        scope_id: &ScopeId,
    ) -> Result<Vec<diskgraph_core::SuspectGroup>, EngineError> {
        let revision = self
            .latest_revision(scope_id)?
            .ok_or_else(|| EngineError::Business(BusinessError::NotIndexed))?;
        let graph = self.load_revision(&revision)?;
        Ok(suspects_of(&graph))
    }
}
/// 仅用文件大小与硬链接身份分组疑似重复。
/// 参数：graph 为一份快照图。
/// 返回：元数据疑似组，不包含内容确认或删除授权。
/// The suspects of one graph, as pure metadata (exported for tests).
pub fn suspects_of(graph: &DiskGraph) -> Vec<diskgraph_core::SuspectGroup> {
    let identities: Vec<Option<String>> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == diskgraph_core::NodeKind::File)
        .map(|node| {
            node.file_identity
                .as_ref()
                .map(|identity| format!("{}:{}", identity.volume_id, identity.file_id))
        })
        .collect();
    let objects: Vec<(u64, u64, Option<&str>)> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == diskgraph_core::NodeKind::File)
        .zip(&identities)
        .map(|(node, identity)| (node.id, node.direct_bytes, identity.as_deref()))
        .collect();
    diskgraph_core::suspect_groups(&objects)
}
