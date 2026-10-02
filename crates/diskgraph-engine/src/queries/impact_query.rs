//! 方向与预算约束的影响遍历。

use super::{ImpactEntry, ImpactResult, Propagation, impact_propagation};
use diskgraph_core::{BudgetTracker, BusinessError, QueryBudget, Relation, TruncationReason};
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
    let started = std::time::Instant::now();
    let mut tracker = BudgetTracker::new(budget)?;
    let mut seen: HashSet<(String, Relation)> = HashSet::new();
    seen.insert((start.to_owned(), Relation::Contains));
    let mut out = Vec::new();
    let mut frontier = vec![start.to_owned()];
    let mut depth = 0usize;
    while !frontier.is_empty() {
        depth += 1;
        if !tracker.allows_depth(depth) {
            break;
        }
        let mut next = Vec::new();
        for entity in &frontier {
            if started.elapsed() >= std::time::Duration::from_millis(budget.deadline_ms) {
                return Ok(ImpactResult {
                    entries: out,
                    truncated: Some(TruncationReason::Deadline),
                });
            }
            let page_limit = budget
                .max_edges
                .saturating_sub(tracker.edges())
                .min(budget.max_nodes.saturating_sub(tracker.nodes()))
                .max(1);
            let (incoming, incoming_more) = neighbours_for(entity, false, page_limit)?;
            if started.elapsed() >= std::time::Duration::from_millis(budget.deadline_ms) {
                return Ok(ImpactResult {
                    entries: out,
                    truncated: Some(TruncationReason::Deadline),
                });
            }
            let (outgoing, outgoing_more) = neighbours_for(entity, true, page_limit)?;
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
                        if *observed_relation != relation
                            || !seen.insert((neighbour.clone(), relation))
                        {
                            continue;
                        }
                        // The JSON envelope adds fixed fields around these
                        // entries; reserve a conservative per-entry overhead.
                        if !tracker.charge_edge()
                            || !tracker.charge_node()
                            || !tracker.charge_bytes(neighbour.len().saturating_add(128))
                        {
                            return Ok(ImpactResult {
                                entries: out,
                                truncated: tracker.truncated(),
                            });
                        }
                        out.push(ImpactEntry {
                            entity_id: neighbour.clone(),
                            relation,
                            depth,
                        });
                        next.push(neighbour.clone());
                    }
                }
            }
            if incoming_more || outgoing_more {
                return Ok(ImpactResult {
                    entries: out,
                    truncated: Some(TruncationReason::EdgeLimit),
                });
            }
        }
        frontier = next;
    }
    Ok(ImpactResult {
        entries: out,
        truncated: tracker.truncated(),
    })
}
