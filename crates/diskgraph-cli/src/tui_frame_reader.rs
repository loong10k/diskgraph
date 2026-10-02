use std::time::Instant;

use diskgraph_engine::EngineError;
use diskgraph_store::SqliteSnapshotStore;

use crate::tui::{Layer, layer_from_nodes};

/// 一帧递归地图的共同读取预算；保留已绘制父块并说明截断原因。
pub struct TuiFrameReader<'a> {
    reader: &'a SqliteSnapshotStore,
    snapshot_id: &'a str,
    deadline: Instant,
    remaining_rows: usize,
    remaining_queries: usize,
    remaining_bytes: usize,
    pub truncation_reason: Option<&'static str>,
    failure: Option<EngineError>,
}

impl<'a> TuiFrameReader<'a> {
    /// 使用该帧已授权的快照和 SQLite 连接；缓存根层同样占节点预算。
    pub fn new(
        reader: &'a SqliteSnapshotStore,
        snapshot_id: &'a str,
        deadline: Instant,
        cached_nodes: usize,
        cached_bytes: usize,
    ) -> Self {
        Self {
            reader,
            snapshot_id,
            deadline,
            remaining_rows: 2048usize.saturating_sub(cached_nodes),
            remaining_queries: 4,
            remaining_bytes: (256 * 1024usize).saturating_sub(cached_bytes),
            truncation_reason: None,
            failure: None,
        }
    }

    /// 取出非预算读取故障；展示适配器须在提交帧之前传播该错误。
    pub fn take_error(&mut self) -> Option<EngineError> {
        self.failure.take()
    }

    /// 计算展示层文本与逐项保留成本；缓存层和嵌套层使用同一口径。
    pub fn display_bytes(layer: &Layer) -> usize {
        layer.name.len()
            + layer
                .children
                .iter()
                .map(|child| {
                    child.name.len()
                        + child.kind.len()
                        + child.category.as_ref().map_or(0, String::len)
                        + 64
                })
                .sum::<usize>()
    }

    /// 检查共同截止时间；已经截断后不再开始额外读取。
    pub fn can_continue(&mut self) -> bool {
        if self.truncation_reason.is_none() && Instant::now() >= self.deadline {
            self.truncation_reason = Some("deadline");
        }
        self.truncation_reason.is_none()
    }

    /// 读取一个子层，父节点及翻页探针也计入整帧预算。
    pub fn load_layer(&mut self, parent_id: u64, page_size: usize) -> Option<Layer> {
        if !self.can_continue() {
            return None;
        }
        if self.remaining_queries == 0 {
            self.truncation_reason = Some("query_budget");
            return None;
        }
        if self.remaining_rows <= 2 {
            self.truncation_reason = Some("node_budget");
            return None;
        }
        let limit = page_size.min(self.remaining_rows - 2);
        self.remaining_queries -= 1;
        // 先预留，不因错误或舍弃结果退回已花费的工作量。
        self.remaining_rows -= limit + 2;
        let page = (|| -> Result<Layer, EngineError> {
            let node =
                self.reader
                    .node(self.snapshot_id, parent_id)?
                    .ok_or(EngineError::Business(
                        diskgraph_core::BusinessError::NotFound,
                    ))?;
            let mut children =
                self.reader
                    .children(self.snapshot_id, parent_id, 0, limit as u64 + 1)?;
            // 不足一页时只计真正解码的父节点和子节点。
            self.remaining_rows += limit + 2 - (children.len() + 1);
            let has_more = children.len() > limit;
            children.truncate(limit);
            Ok(layer_from_nodes(node, children, 0, has_more))
        })();
        match page {
            Ok(layer) => {
                let bytes = Self::display_bytes(&layer);
                if bytes > self.remaining_bytes {
                    self.truncation_reason = Some("byte_budget");
                    return None;
                }
                self.remaining_bytes -= bytes;
                if !self.can_continue() {
                    return None;
                }
                if layer.has_more {
                    self.truncation_reason = Some(if limit < page_size {
                        "node_budget"
                    } else {
                        "nested_page"
                    });
                    return None;
                }
                Some(layer)
            }
            Err(EngineError::Store(error)) if error.is_interrupted() => {
                self.truncation_reason = Some("deadline");
                None
            }
            Err(error) => {
                self.truncation_reason = Some("read_error");
                self.failure = Some(error);
                None
            }
        }
    }
}
