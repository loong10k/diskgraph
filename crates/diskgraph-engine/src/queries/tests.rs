use super::*;
use diskgraph_core::{BusinessError, DiskGraph, DiskNode, QueryBudget, Relation, TruncationReason};
use diskgraph_core::{DiskSnapshot, NodeKind, ResourceLocator, ScanCoverage, ScanSettings};
use std::collections::HashMap;

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
    let from_project = impact(&by_source, &by_target, "project", QueryBudget::default()).unwrap();
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
