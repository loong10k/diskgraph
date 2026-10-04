//! 方向与预算约束的影响遍历。

use super::{ImpactEntry, ImpactResult, Propagation, impact_propagation};
use diskgraph_core::{BusinessError, QueryBudget, Relation, TruncationReason};
use std::collections::{HashMap, HashSet};

/// 按关系方向执行有界影响遍历。
/// 参数：双向邻接表、start 与 budget 为查询上下文。
/// 返回：影响条目或无效预算；不授予执行权限。
/// Bounded forward impact: which entities a change here would plausibly reach,
/// following each relation's own direction. This is an observation surface,
/// never an execution authorization (C15).
pub fn impact(
    edges_by_source: &HashMap<String, Vec<(String, Relation)>>,
    edges_by_target: &HashMap<String, Vec<(String, Relation)>>,
    start: &str,
    budget: QueryBudget,
) -> Result<Vec<ImpactEntry>, BusinessError> {
    impact_bounded(edges_by_source, edges_by_target, start, budget).map(|result| result.entries)
}
/// 返回影响遍历条目及明确完成诊断。
/// 参数：双向邻接表、start 与 budget 为查询上下文。
/// 返回：影响结果或预算参数失败。
/// Runs impact with explicit completeness diagnostics for clients that need to
/// distinguish the first bounded page from an exhaustive graph answer.
pub fn impact_bounded(
    edges_by_source: &HashMap<String, Vec<(String, Relation)>>,
    edges_by_target: &HashMap<String, Vec<(String, Relation)>>,
    start: &str,
    budget: QueryBudget,
) -> Result<ImpactResult, BusinessError> {
    impact_bounded_with_neighbors(start, budget, |entity, outgoing, _limit| {
        let map = if outgoing {
            edges_by_source
        } else {
            edges_by_target
        };
        Ok((map.get(entity).cloned().unwrap_or_default(), false))
    })
}
/// 在分页邻接 reader 上执行有界方向遍历。
/// 参数：start/budget 为约束，neighbours_for 返回邻接页及更多标志。
/// 返回：泛型结果或 reader 错误，未读关系明确为截断。
/// Bounded traversal over a per-entity edge reader. The fetcher receives a
/// maximum page size and returns (neighbour, relation) pairs plus a `more`
/// flag. If a page leaves unread edges, this answer is explicitly partial.
pub fn impact_bounded_with_neighbors<E, F>(
    start: &str,
    budget: QueryBudget,
    mut neighbours_for: F,
) -> Result<ImpactResult, E>
where
    E: From<BusinessError>,
    F: FnMut(&str, bool, usize) -> Result<(Vec<(String, Relation)>, bool), E>,
{
    let deadline = diskgraph_core::query_deadline(budget)?;
    let mut reads = diskgraph_core::QueryReadBudget::new(budget, deadline)?;
    impact_with_budget(
        start,
        budget,
        &mut reads,
        |entity, outgoing, limit, reads| {
            let (neighbours, more) = neighbours_for(entity, outgoing, limit)?;
            let mut page = Vec::new();
            let mut unread = more;
            for (neighbour, relation) in neighbours {
                if page.len() >= limit || !reads.admit(0, 1, neighbour.len()) {
                    unread = true;
                    break;
                }
                page.push((neighbour, relation));
            }
            Ok((page, unread))
        },
    )
}

/// 按实际读取账本及同一期限遍历已授权邻接。
/// 参数：start/budget/deadline 和 reader 为同一次请求。
/// 返回：条目及截断；reader 真实错误保持原样。
pub(crate) fn impact_with_budget<E, F>(
    start: &str,
    budget: QueryBudget,
    reads: &mut diskgraph_core::QueryReadBudget,
    mut neighbours_for: F,
) -> Result<ImpactResult, E>
where
    E: From<BusinessError>,
    F: FnMut(
        &str,
        bool,
        usize,
        &mut diskgraph_core::QueryReadBudget,
    ) -> Result<(Vec<(String, Relation)>, bool), E>,
{
    let deadline = reads.deadline();
    let mut seen: HashSet<(String, Relation)> = HashSet::new();
    seen.insert((start.to_owned(), Relation::Contains));
    let mut answer = ImpactResult {
        entries: Vec::new(),
        truncated: None,
    };
    let mut frontier = vec![start.to_owned()];
    let mut depth = 0usize;
    let response_cap = budget.max_response_bytes.saturating_sub(2048);
    let mut response_bytes = 0usize;
    'walk: while !frontier.is_empty() {
        if std::time::Instant::now() >= deadline {
            answer.truncated = Some(TruncationReason::Deadline);
            break;
        }
        depth += 1;
        if depth > budget.max_depth {
            answer.truncated = Some(TruncationReason::DepthLimit);
            break;
        }
        let mut next = Vec::new();
        for entity in &frontier {
            if std::time::Instant::now() >= deadline {
                answer.truncated = Some(TruncationReason::Deadline);
                break 'walk;
            }
            let limit = reads
                .remaining_edges()
                .min(budget.max_nodes.saturating_sub(answer.entries.len()));
            let (incoming, incoming_more) = neighbours_for(entity, false, limit, reads)?;
            if std::time::Instant::now() >= deadline {
                answer.truncated = Some(TruncationReason::Deadline);
                break 'walk;
            }
            let limit = reads
                .remaining_edges()
                .min(budget.max_nodes.saturating_sub(answer.entries.len()));
            let (outgoing, outgoing_more) = if reads.stopped().is_none() {
                neighbours_for(entity, true, limit, reads)?
            } else {
                (Vec::new(), true)
            };
            if std::time::Instant::now() >= deadline {
                answer.truncated = Some(TruncationReason::Deadline);
                break 'walk;
            }
            for relation in [
                Relation::Contains,
                Relation::Declares,
                Relation::OwnedByProject,
                Relation::OwnedByApplication,
                Relation::UsedByProcess,
                Relation::RebuildableBy,
                Relation::ProtectedBy,
                Relation::SameContentAs,
            ] {
                let Some(propagation) = impact_propagation(relation) else {
                    continue;
                };
                for (direction, neighbours) in [
                    (Propagation::Incoming, incoming.as_slice()),
                    (Propagation::Outgoing, outgoing.as_slice()),
                ] {
                    if propagation != direction && propagation != Propagation::Both {
                        continue;
                    }
                    for (neighbour, observed_relation) in neighbours {
                        if std::time::Instant::now() >= deadline {
                            answer.truncated = Some(TruncationReason::Deadline);
                            break 'walk;
                        }
                        if *observed_relation != relation
                            || seen.contains(&(neighbour.clone(), relation))
                        {
                            continue;
                        }
                        if answer.entries.len() >= budget.max_nodes {
                            // 输出节点门禁独立于已解码边余额；同时耗尽保留旧 EdgeLimit 优先级。
                            answer.truncated = Some(if reads.remaining_edges() == 0 {
                                TruncationReason::EdgeLimit
                            } else {
                                TruncationReason::NodeLimit
                            });
                            break 'walk;
                        }
                        let entry = ImpactEntry {
                            entity_id: neighbour.clone(),
                            relation,
                            depth,
                        };
                        let cost = diskgraph_core::measure_json_bounded(
                            &entry,
                            response_cap.saturating_sub(response_bytes),
                        )
                        .map_err(|_| BusinessError::InternalError)?;
                        let Some(total) = cost
                            .and_then(|n| response_bytes.checked_add(n))
                            .and_then(|n| n.checked_add(1))
                            .filter(|n| *n <= response_cap)
                        else {
                            answer.truncated = Some(TruncationReason::ByteLimit);
                            break 'walk;
                        };
                        response_bytes = total;
                        seen.insert((neighbour.clone(), relation));
                        answer.entries.push(entry);
                        next.push(neighbour.clone());
                    }
                }
            }
            if incoming_more || outgoing_more {
                answer.truncated = Some(reads.stopped().unwrap_or(
                    if answer.entries.len() >= budget.max_nodes && reads.remaining_edges() > 0 {
                        TruncationReason::NodeLimit
                    } else {
                        TruncationReason::EdgeLimit
                    },
                ));
                break 'walk;
            }
        }
        frontier = next;
    }
    if std::time::Instant::now() >= deadline {
        answer.truncated = Some(TruncationReason::Deadline);
    }
    if diskgraph_core::measure_json_bounded(&answer, budget.max_response_bytes)
        .map_err(|_| BusinessError::InternalError)?
        .is_none()
    {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(answer)
}
