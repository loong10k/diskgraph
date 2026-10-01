//! Bounded graph queries (P2 tasks 3.7 / 3.8, specs Q-01 / Q-02 / Q-03 /
//! Q-05): search, explore, and impact. Every traversal is budget-bounded and
//! reports truncation explicitly; `impact` follows relation-specific
//! propagation instead of undirected expansion, and never grants execution.

use std::collections::{HashMap, HashSet};

use diskgraph_core::{
    BudgetTracker, BusinessError, CursorContext, DiskGraph, DiskNode, Incompatibility, QueryBudget,
    Relation, TruncationReason,
};

/// How a relation's influence propagates when asking "what does this affect?".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Propagation {
    /// Edges pointing at the target (what depends on it).
    Incoming,
    /// Edges pointing away from the target (what it depends on).
    Outgoing,
    /// Both directions, de-duplicated per (entity, relation) pair.
    Both,
}

/// The direction `impact` traverses for each relation. Relations absent from
/// this table never propagate, so an undirected walk cannot happen by accident.
pub fn impact_propagation(relation: Relation) -> Option<Propagation> {
    Some(match relation {
        // Build artifacts of a project: touching the project affects its outputs.
        Relation::OwnedByProject | Relation::Contains => Propagation::Incoming,
        // What a project owns is what it rebuilds into.
        Relation::RebuildableBy | Relation::Declares => Propagation::Outgoing,
        Relation::OwnedByApplication | Relation::UsedByProcess => Propagation::Both,
        // Protection and content identity never propagate as "affected".
        Relation::ProtectedBy | Relation::SameContentAs => return None,
    })
}

/// Name/path pattern search over one revision. Matching is a pattern lookup,
/// never an intent guess; ambiguity is returned as several candidates.
pub fn search_nodes<'a>(
    graph: &'a DiskGraph,
    pattern: &str,
    offset: u64,
    limit: usize,
    budget: QueryBudget,
) -> (Vec<&'a DiskNode>, Option<u64>) {
    let mut tracker = match BudgetTracker::new(budget) {
        Ok(tracker) => tracker,
        Err(_) => return (Vec::new(), None),
    };
    let needle = pattern.to_lowercase();
    let mut matches: Vec<&DiskNode> = graph
        .nodes
        .iter()
        .filter(|node| {
            node.name.to_lowercase().contains(&needle)
                || match &node.locator {
                    diskgraph_core::ResourceLocator::NativePath(path) => {
                        path.to_lowercase().contains(&needle)
                    }
                    diskgraph_core::ResourceLocator::DocumentUri(uri) => {
                        uri.to_lowercase().contains(&needle)
                    }
                }
        })
        .collect();
    matches.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let mut out: Vec<&DiskNode> = Vec::new();
    let total = matches.len() as u64;
    let mut next_offset = offset;
    for (index, node) in matches.into_iter().enumerate() {
        if (index as u64) < offset {
            continue;
        }
        if out.len() >= limit || !tracker.charge_node() {
            next_offset = offset + out.len() as u64;
            break;
        }
        out.push(node);
    }
    // A page that stops short of the total still offers a continuation.
    let consumed = offset + out.len() as u64;
    let next = if consumed < total {
        Some(consumed)
    } else {
        None
    };
    let _ = next_offset;
    (out, next)
}

/// A bounded directory/relation summary: the node plus its direct children
/// ranked by observed size, with explicit coverage and truncation.
pub struct ExploreSummary<'a> {
    pub node: &'a DiskNode,
    pub children: Vec<&'a DiskNode>,
    pub coverage: &'a diskgraph_core::ScanCoverage,
    pub truncated: Option<TruncationReason>,
}

pub fn explore<'a>(graph: &'a DiskGraph, node_id: u64, budget: QueryBudget) -> ExploreSummary<'a> {
    let mut tracker = BudgetTracker::new(budget).unwrap_or_else(|_| {
        BudgetTracker::new(QueryBudget::default()).expect("default budget is valid")
    });
    let node = graph
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .unwrap_or(&graph.nodes[0]);
    let max_children = tracker.budget().max_nodes;
    let ranked = graph.top(node.id, max_children);
    let hit_cap = ranked.len() >= max_children;
    let mut children: Vec<&DiskNode> = Vec::new();
    for child in &ranked {
        if !tracker.charge_node() {
            break;
        }
        children.push(*child);
    }
    ExploreSummary {
        node,
        coverage: &graph.snapshot.coverage,
        truncated: tracker.truncated().or(if hit_cap {
            Some(TruncationReason::NodeLimit)
        } else {
            None
        }),
        children,
    }
}

/// One affected entity with the relation and depth that reached it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImpactEntry {
    pub entity_id: String,
    pub relation: Relation,
    pub depth: usize,
}

/// Impact entries together with the reason traversal stopped before completion.
pub struct ImpactResult {
    pub entries: Vec<ImpactEntry>,
    pub truncated: Option<TruncationReason>,
}

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

/// Renders an incompatibility reason as the stable wire name.
pub fn incompatibility_name(reason: Incompatibility) -> &'static str {
    match reason {
        Incompatibility::DifferentRoot => "different_root",
        Incompatibility::DifferentVolume => "different_volume",
        Incompatibility::UnknownVolume => "unknown_volume",
        Incompatibility::DifferentSettings => "different_settings",
        Incompatibility::OutOfOrder => "out_of_order",
        Incompatibility::IncompleteCoverage => "incomplete_coverage",
    }
}

/// Builds the cursor context for one query identity.
pub fn cursor_context<'a>(
    principal_binding: &'a str,
    scope_id: &'a str,
    revision_id: &'a str,
) -> CursorContext<'a> {
    CursorContext {
        principal_binding,
        scope_id,
        revision_id,
        filter_binding: "",
        sort_binding: "size_desc,name_asc",
        policy_version: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diskgraph_core::{DiskSnapshot, NodeKind, ResourceLocator, ScanCoverage, ScanSettings};

    fn node(id: u64, parent: Option<u64>, name: &str, bytes: u64) -> DiskNode {
        DiskNode {
            id,
            parent_id: parent,
            locator: ResourceLocator::NativePath(format!("/tmp/{name}")),
            name: name.to_owned(),
            kind: NodeKind::Directory,
            subtree_bytes: bytes,
            size_known: true,
            direct_bytes: 0,
            files: 0,
            directories: 1,
            modified_unix_seconds: None,
            file_identity: None,
            category_hint: None,
            reclaim_hint: None,
            read_error: false,
        }
    }

    fn graph() -> DiskGraph {
        DiskGraph {
            snapshot: DiskSnapshot {
                id: "snap".into(),
                root: ResourceLocator::NativePath("/tmp".into()),
                volume_id: Some("v".into()),
                captured_at_unix_ms: 1,
                settings: ScanSettings {
                    apparent_size: false,
                    follow_links: false,
                    include_hidden: true,
                    one_filesystem: true,
                    max_depth: None,
                    dedup_hardlinks: true,
                },
                coverage: ScanCoverage {
                    complete: true,
                    unreadable_nodes: 0,
                    depth_limited: false,
                },
            },
            nodes: vec![
                node(1, None, "root", 300),
                node(2, Some(1), "Cargo.toml", 10),
                node(3, Some(1), "target", 200),
                node(4, Some(1), "notes", 90),
            ],
            evidence: vec![],
        }
    }

    #[test]
    fn search_matches_names_and_paths_with_stable_order_and_paging() {
        let graph = graph();
        let (first, next) = search_nodes(&graph, "t", 0, 2, QueryBudget::default());
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].name, "Cargo.toml");
        assert!(next.is_some(), "a second page must be offered");
        let (second, last) = search_nodes(&graph, "t", 2, 2, QueryBudget::default());
        assert!(!second.is_empty());
        assert!(last.is_none());
    }

    #[test]
    fn explore_summarises_children_with_coverage() {
        let graph = graph();
        let summary = explore(&graph, 1, QueryBudget::default());
        assert_eq!(summary.node.id, 1);
        assert_eq!(summary.children.len(), 3);
        assert_eq!(summary.children[0].name, "target");
        assert!(summary.coverage.complete);
        assert!(summary.truncated.is_none());
    }

    #[test]
    fn explore_respects_a_tight_node_budget() {
        let graph = graph();
        let summary = explore(
            &graph,
            1,
            QueryBudget {
                max_nodes: 1,
                ..QueryBudget::default()
            },
        );
        assert_eq!(summary.children.len(), 1);
        assert_eq!(summary.truncated, Some(TruncationReason::NodeLimit));
    }

    #[test]
    fn impact_follows_relation_direction_not_undirected_reach() {
        // project <-[owned_by_project]- target ; target -[rebuildable_by]-> recipe
        let mut by_source: HashMap<String, Vec<(String, Relation)>> = HashMap::new();
        let mut by_target: HashMap<String, Vec<(String, Relation)>> = HashMap::new();
        by_target
            .entry("project".into())
            .or_default()
            .push(("target".into(), Relation::OwnedByProject));
        by_source
            .entry("target".into())
            .or_default()
            .push(("recipe".into(), Relation::RebuildableBy));

        // From the project: its output at depth 1, and the recipe only
        // reachable through that output at depth 2 (never directly).
        let from_project =
            impact(&by_source, &by_target, "project", QueryBudget::default()).unwrap();
        let target_entry = from_project
            .iter()
            .find(|entry| entry.entity_id == "target")
            .expect("the owned output is directly affected");
        assert_eq!(target_entry.depth, 1);
        let recipe_entry = from_project
            .iter()
            .find(|entry| entry.entity_id == "recipe")
            .expect("the recipe is reachable through its output");
        assert_eq!(recipe_entry.depth, 2);

        // From the output the direction flips: the recipe is reachable, the
        // project it belongs to is not (that edge points the other way).
        let from_target = impact(&by_source, &by_target, "target", QueryBudget::default()).unwrap();
        assert!(from_target.iter().any(|entry| entry.entity_id == "recipe"));
        assert!(!from_target.iter().any(|entry| entry.entity_id == "project"));
    }

    #[test]
    fn impact_stops_at_the_edge_budget() {
        let mut by_target: HashMap<String, Vec<(String, Relation)>> = HashMap::new();
        by_target
            .entry("project".into())
            .or_default()
            .push(("a".into(), Relation::OwnedByProject));
        by_target
            .entry("project".into())
            .or_default()
            .push(("b".into(), Relation::OwnedByProject));
        let empty: HashMap<String, Vec<(String, Relation)>> = HashMap::new();
        let budget = QueryBudget {
            max_edges: 1,
            ..QueryBudget::default()
        };
        let entries = impact(&empty, &by_target, "project", budget).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn impact_reports_that_an_edge_budget_made_the_answer_partial() {
        let mut by_target: HashMap<String, Vec<(String, Relation)>> = HashMap::new();
        by_target.insert(
            "project".into(),
            vec![
                ("a".into(), Relation::OwnedByProject),
                ("b".into(), Relation::OwnedByProject),
            ],
        );
        let empty = HashMap::new();
        let answer = impact_bounded(
            &empty,
            &by_target,
            "project",
            QueryBudget {
                max_edges: 1,
                ..QueryBudget::default()
            },
        )
        .unwrap();
        assert_eq!(answer.entries.len(), 1);
        assert_eq!(answer.truncated, Some(TruncationReason::EdgeLimit));
    }

    #[test]
    fn impact_uses_the_declared_bidirectional_relations() {
        let mut by_source = HashMap::new();
        let mut by_target = HashMap::new();
        by_source.insert(
            "resource".into(),
            vec![("application".into(), Relation::OwnedByApplication)],
        );
        by_target.insert(
            "application".into(),
            vec![("resource".into(), Relation::OwnedByApplication)],
        );
        assert!(
            impact(&by_source, &by_target, "resource", QueryBudget::default())
                .unwrap()
                .iter()
                .any(|entry| entry.entity_id == "application")
        );
        assert!(
            impact(
                &by_source,
                &by_target,
                "application",
                QueryBudget::default()
            )
            .unwrap()
            .iter()
            .any(|entry| entry.entity_id == "resource")
        );
    }

    #[test]
    fn paged_impact_does_not_claim_completion_when_edges_remain_unread() {
        let result = impact_bounded_with_neighbors::<BusinessError, _>(
            "root",
            QueryBudget::default(),
            |_entity, outgoing, limit| {
                assert!(limit <= QueryBudget::default().max_nodes);
                if outgoing {
                    Ok((Vec::new(), false))
                } else {
                    Ok((vec![("ignored".into(), Relation::ProtectedBy)], true))
                }
            },
        )
        .unwrap();
        assert!(result.entries.is_empty());
        assert_eq!(result.truncated, Some(TruncationReason::EdgeLimit));
    }

    #[test]
    fn impact_deadline_stops_before_fetching_the_second_direction() {
        let result = impact_bounded_with_neighbors::<BusinessError, _>(
            "root",
            QueryBudget {
                deadline_ms: 1,
                ..QueryBudget::default()
            },
            |_entity, outgoing, _limit| {
                assert!(!outgoing, "deadline should stop the second query");
                std::thread::sleep(std::time::Duration::from_millis(20));
                Ok((Vec::new(), false))
            },
        )
        .unwrap();
        assert_eq!(result.truncated, Some(TruncationReason::Deadline));
    }

    #[test]
    fn impact_rejects_a_zero_budget() {
        let budget = QueryBudget {
            max_edges: 0,
            ..QueryBudget::default()
        };
        let empty: HashMap<String, Vec<(String, Relation)>> = HashMap::new();
        assert!(matches!(
            impact(&empty, &empty, "x", budget),
            Err(BusinessError::InvalidArgument)
        ));
    }

    #[test]
    fn protected_and_same_content_relations_never_propagate() {
        assert_eq!(impact_propagation(Relation::ProtectedBy), None);
        assert_eq!(impact_propagation(Relation::SameContentAs), None);
        assert!(impact_propagation(Relation::Contains).is_some());
    }
}
