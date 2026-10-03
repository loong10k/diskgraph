use crate::{BusinessError, QueryBudget, TruncationReason};
use std::time::Instant;

/// 请求局部的实际解码数量与 SQLite 借用字段准入账本。
/// 来源：DiskGraph 原生 Rust D23 查询预算；无 Java 对应对象。
/// raw 字节独立于响应编码额度，不对同一响应重复收费；不限制 SQLite 页缓存/RSS。
pub struct QueryReadBudget {
    budget: QueryBudget,
    deadline: Instant,
    nodes: usize,
    edges: usize,
    raw_bytes: usize,
    stopped: Option<TruncationReason>,
}

impl QueryReadBudget {
    /// 建立共享期限上的读取账本。
    /// 参数：budget 为正数 typed 额度，deadline 为首次准备前生成的绝对期限。
    /// 返回：空账本或无效预算。
    pub fn new(budget: QueryBudget, deadline: Instant) -> Result<Self, BusinessError> {
        Ok(Self {
            budget: budget.validated()?,
            deadline,
            nodes: 0,
            edges: 0,
            raw_bytes: 0,
            stopped: None,
        })
    }

    /// 在拥有/解码字段之前原子计入本行成本。
    /// 参数：nodes/edges 为实际解码增量，raw_bytes 为 SQLite 借用字段总长。
    /// 返回：准入成功；超限保留旧计数并记录原因。
    pub fn admit(&mut self, nodes: usize, edges: usize, raw_bytes: usize) -> bool {
        if !self.check() {
            return false;
        }
        let Some(next_nodes) = self
            .nodes
            .checked_add(nodes)
            .filter(|n| *n <= self.budget.max_nodes)
        else {
            self.stop(TruncationReason::NodeLimit);
            return false;
        };
        let Some(next_edges) = self
            .edges
            .checked_add(edges)
            .filter(|n| *n <= self.budget.max_edges)
        else {
            self.stop(TruncationReason::EdgeLimit);
            return false;
        };
        let Some(next_raw) = self
            .raw_bytes
            .checked_add(raw_bytes)
            .filter(|n| *n <= self.budget.max_response_bytes)
        else {
            self.stop(TruncationReason::ByteLimit);
            return false;
        };
        self.nodes = next_nodes;
        self.edges = next_edges;
        self.raw_bytes = next_raw;
        true
    }

    /// 每次读取和空结果终态均检查同一期限。
    /// 参数：无。返回：尚未停止为 true；到期总是明确 Deadline。
    pub fn check(&mut self) -> bool {
        if Instant::now() >= self.deadline {
            self.stopped = Some(TruncationReason::Deadline);
        }
        self.stopped.is_none()
    }
    /// 标记当前未读取部分。
    /// 参数：reason 为真实预算原因。返回：无；优先保留此前停止原因。
    pub fn stop(&mut self, reason: TruncationReason) {
        self.stopped.get_or_insert(reason);
    }
    /// 读取截断原因。
    /// 参数：无。返回：当前原因，尚未停止则 None。
    pub fn stopped(&self) -> Option<TruncationReason> {
        self.stopped
    }
    /// 获取共享期限。
    /// 参数：无。返回：原始 Instant，不重新计时。
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    /// 获取当前剩余边解码数量。
    /// 参数：无。返回：可准入的边数，重复/不传播项也已计费。
    pub fn remaining_edges(&self) -> usize {
        self.budget.max_edges - self.edges
    }
    /// 获取当前剩余借用字段字节额度。
    /// 参数：无。返回：剩余原始字段字节，独立于实际 JSON 响应计量。
    pub fn remaining_raw_bytes(&self) -> usize {
        self.budget.max_response_bytes - self.raw_bytes
    }
}
