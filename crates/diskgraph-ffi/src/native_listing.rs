//! 旧列表与会话部分页共享有界读取，分别保留原 wire 语义；来源：D26 / Q-02。
use crate::bounded_limit;
use diskgraph_core::{QueryBudget, QueryReadBudget, TruncationReason, measure_json_bounded};
use diskgraph_store::SqliteSnapshotStore;
use serde_json::{Value, json};
use std::time::Instant;

/// 读取旧完整列表；参数固定页和原请求期限，返回节点与是否存在下一页或预算错误。
fn legacy_page(
    store: &SqliteSnapshotStore,
    snapshot: &str,
    parent: u64,
    offset: u64,
    limit: u32,
    deadline: Instant,
) -> Result<(Vec<diskgraph_core::DiskNode>, bool), String> {
    let limit = bounded_limit(limit)?;
    let mut budget = QueryReadBudget::new(
        QueryBudget {
            max_nodes: limit as usize,
            ..QueryBudget::default()
        },
        deadline,
    )
    .map_err(|e| e.to_string())?;
    let (nodes, more, _) = store
        .children_with_budget(
            snapshot,
            parent,
            offset,
            u64::from(limit),
            false,
            &mut budget,
        )
        .map_err(|e| e.to_string())?;
    if budget.stopped().is_some() {
        return Err(
            "budget_exceeded: incomplete legacy page; use the session API for partial results"
                .into(),
        );
    }
    Ok((nodes, more))
}

/// 保留旧top数组；参数为固定父目录/limit/期限，返回完整数组或错误。
/// 读取旧 top 完整数组。
/// 参数：store/snapshot 为快照，parent 为父节点，limit 为上限，deadline 为固定期限。
/// 返回：节点数组 JSON 值或超预算错误。
pub(crate) fn top(
    store: &SqliteSnapshotStore,
    snapshot: &str,
    parent: u64,
    limit: u32,
    deadline: Instant,
) -> Result<Value, String> {
    let (nodes, _) = legacy_page(store, snapshot, parent, 0, limit, deadline)?;
    Ok(json!(nodes))
}

/// 保留旧children分页对象；参数为固定目录页/期限，next_offset按真实返回条数推进。
/// 读取旧 children 完整页。
/// 参数：store/snapshot 为快照，parent 为父节点，offset/limit 为分页，deadline 为期限。
/// 返回：items 和按实际条数推进的 next_offset 对象或错误。
pub(crate) fn children(
    store: &SqliteSnapshotStore,
    snapshot: &str,
    parent: u64,
    offset: u64,
    limit: u32,
    deadline: Instant,
) -> Result<Value, String> {
    let (nodes, more) = legacy_page(store, snapshot, parent, offset, limit, deadline)?;
    let next = more
        .then(|| {
            offset
                .checked_add(nodes.len() as u64)
                .ok_or("invalid_argument: offset overflow")
        })
        .transpose()?;
    Ok(json!({"items":nodes,"next_offset":next}))
}

/// 会话保留有界前缀和精确未知大小数；参数为固定页/期限，返回带停止原因的部分页。
/// 读取会话目录页并保留有界前缀。
/// 参数：store/snapshot 为快照，parent 为父节点，offset/limit 为分页，deadline 为期限。
/// 返回：含精确未知大小数和停止原因的部分页对象。
pub(crate) fn session_children(
    store: &SqliteSnapshotStore,
    snapshot: &str,
    parent: u64,
    offset: u64,
    limit: u32,
    deadline: Instant,
) -> Result<Value, String> {
    let limit = bounded_limit(limit)?.min(100);
    let mut budget =
        QueryReadBudget::new(QueryBudget::default(), deadline).map_err(|e| e.to_string())?;
    let (nodes, more, unknown) = store
        .children_with_budget(
            snapshot,
            parent,
            offset,
            u64::from(limit),
            true,
            &mut budget,
        )
        .map_err(|e| e.to_string())?;
    let mut reason = budget.stopped().map(|reason| match reason {
        TruncationReason::ByteLimit => "raw_byte_limit",
        TruncationReason::Deadline => "deadline",
        _ => "node_limit",
    });
    let cap = QueryBudget::default()
        .max_response_bytes
        .saturating_sub(2048);
    let mut items = Vec::new();
    let mut remaining = cap;
    for node in nodes {
        let Some(cost) = measure_json_bounded(&node, remaining).map_err(|e| e.to_string())? else {
            reason = Some("response_byte_limit");
            break;
        };
        remaining = remaining.saturating_sub(cost.saturating_add(1));
        items.push(node);
    }
    if reason.is_some() && items.is_empty() {
        return Err("budget_exceeded: no complete node fits in the page".into());
    }
    let next = (more || reason.is_some())
        .then(|| {
            offset
                .checked_add(items.len() as u64)
                .ok_or("invalid_argument: offset overflow")
        })
        .transpose()?;
    Ok(
        json!({"items":items,"next_offset":next,"unknown_size_count":unknown,
        "complete":next.is_none(),"truncated":reason}),
    )
}
