use std::time::Instant;

use diskgraph_core::{QueryReadBudget, TruncationReason};
use diskgraph_engine::EngineError;
use diskgraph_store::{SqliteSnapshotStore, StoreError};

use crate::tui::Layer;
use crate::tui_request::{TUI_DISPLAY_BYTES, TUI_NODE_LIMIT, TUI_PREPARATION_BYTES, TuiRequest};

/// 一帧递归地图的共同读取预算；保留已绘制父块并说明截断原因。
/// 来源：DiskGraph 原生 Rust Q-08 / 13.6；无 Java 对应对象。
pub struct TuiFrameReader<'a> {
    reader: &'a SqliteSnapshotStore,
    snapshot_id: &'a str,
    deadline: Instant,
    remaining_rows: usize,
    remaining_queries: usize,
    remaining_bytes: usize,
    read_budget: QueryReadBudget,
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
            remaining_rows: TUI_NODE_LIMIT.saturating_sub(cached_nodes),
            remaining_queries: 4,
            remaining_bytes: TUI_DISPLAY_BYTES.saturating_sub(cached_bytes),
            // 缓存本帧没有再解码，原始输入只记录本次真正读取的必要字段。
            read_budget: TuiRequest::read_budget(deadline),
            truncation_reason: None,
            failure: None,
        }
    }

    /// 取出非预算读取故障；展示适配器须在提交帧之前传播该错误。
    pub fn take_error(&mut self) -> Option<EngineError> {
        self.failure.take()
    }

    /// 本帧实际准入的新 SQL 原始字段成本；不包含缓存或 SQLite C 分配。
    /// 参数：无。返回：借用字段累计字节，独立于展示留存成本。
    pub fn preparation_bytes(&self) -> usize {
        TUI_PREPARATION_BYTES - self.read_budget.remaining_raw_bytes()
    }

    /// 本帧缓存与已经保留的嵌套展示成本。
    /// 参数：无。返回：已有展示计量口径的字节，不是原始输入或 RSS。
    pub fn retained_display_bytes(&self) -> usize {
        TUI_DISPLAY_BYTES - self.remaining_bytes
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
            self.read_budget.check();
        }
        self.truncation_reason.is_none()
    }

    /// 读取一个子层，父节点与子页累计计费，常量存在探针共用原期限而不解码节点。
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
        if self.remaining_bytes == 0 {
            self.truncation_reason = Some("byte_budget");
            return None;
        }
        let limit = page_size.min(self.remaining_rows - 2);
        self.remaining_queries -= 1;
        let before = self.read_budget.nodes_read();
        let page = TuiRequest::read_layer(
            self.reader,
            self.snapshot_id,
            parent_id,
            0,
            limit,
            &mut self.read_budget,
        );
        // 失败和舍弃页面同样保留实际准备成本；页外存在探针不当成已解码节点。
        self.remaining_rows = self
            .remaining_rows
            .saturating_sub(self.read_budget.nodes_read() - before);
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
            Err(EngineError::Store(StoreError::BudgetExceeded))
                if self.read_budget.stopped().is_some() =>
            {
                self.truncation_reason = Some(match self.read_budget.stopped().unwrap() {
                    TruncationReason::Deadline => "deadline",
                    TruncationReason::ByteLimit => "raw_byte_budget",
                    TruncationReason::NodeLimit => "node_budget",
                    _ => "query_budget",
                });
                None
            }
            Err(EngineError::Store(error))
                if error.is_interrupted() && !self.read_budget.check() =>
            {
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
