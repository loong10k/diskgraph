//! 保存已遍历影响条目及未完成原因。

use super::ImpactEntry;
use diskgraph_core::TruncationReason;

/// 保存已遍历影响条目及未完成原因。
/// 来源：原生 Rust diskgraph-engine::ImpactResult。
/// Impact entries together with the reason traversal stopped before completion.
pub struct ImpactResult {
    pub entries: Vec<ImpactEntry>,
    pub truncated: Option<TruncationReason>,
}

impl serde::Serialize for ImpactResult {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ImpactResult", 4)?;
        state.serialize_field("entries", &self.entries)?;
        state.serialize_field("complete", &self.truncated.is_none())?;
        state.serialize_field("truncated", &self.truncated)?;
        state.serialize_field("grants_execution", &false)?;
        state.end()
    }
}
