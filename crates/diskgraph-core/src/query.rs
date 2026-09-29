use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use crate::{DiskGraph, DiskNode, EvidenceEdge, EvidenceRelation, ResourceLocator};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<'a> {
    pub items: Vec<&'a DiskNode>,
    pub next_offset: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Growth<'a> {
    pub before: &'a DiskNode,
    pub after: &'a DiskNode,
    pub delta_bytes: i128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeExplanation<'a> {
    pub node: &'a DiskNode,
    pub evidence: Vec<&'a EvidenceEdge>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate<'a> {
    pub node: &'a DiskNode,
    pub evidence: Vec<&'a EvidenceEdge>,
}

impl DiskGraph {
    /// Largest immediate children of a node; this never returns a delete plan.
    pub fn top(&self, parent_id: u64, limit: usize) -> Vec<&DiskNode> {
        let mut children: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.parent_id == Some(parent_id))
            .collect();
        children.sort_by(|a, b| {
            b.subtree_bytes
                .cmp(&a.subtree_bytes)
                .then_with(|| a.name.cmp(&b.name))
        });
        children.truncate(limit);
        children
    }

    /// A bounded page of immediate children, ordered by observed size.
    pub fn children(&self, parent_id: u64, offset: usize, limit: usize) -> Page<'_> {
        let all = self.top(parent_id, usize::MAX);
        let end = offset.saturating_add(limit).min(all.len());
        let items = all.get(offset..end).unwrap_or_default().to_vec();
        Page {
            items,
            next_offset: (end < all.len()).then_some(end),
        }
    }

    /// Explain a node through recorded evidence, not a generated assertion.
    pub fn explain(&self, node_id: u64) -> Option<NodeExplanation<'_>> {
        let node = self.nodes.iter().find(|node| node.id == node_id)?;
        let evidence = self
            .evidence
            .iter()
            .filter(|edge| edge.node_id == node_id)
            .collect();
        Some(NodeExplanation { node, evidence })
    }

    /// Match an unchanged locator across compatible complete snapshots.
    /// Renames and inaccessible paths are intentionally not inferred.
    pub fn growth<'a>(
        &'a self,
        previous: &'a Self,
        locator: &ResourceLocator,
    ) -> Option<Growth<'a>> {
        if self.snapshot.root != previous.snapshot.root
            || self.snapshot.volume_id != previous.snapshot.volume_id
            || self.snapshot.volume_id.is_none()
            || self.snapshot.settings != previous.snapshot.settings
            || self.snapshot.captured_at_unix_ms < previous.snapshot.captured_at_unix_ms
            || !self.snapshot.coverage.complete
            || !previous.snapshot.coverage.complete
        {
            return None;
        }
        let before = previous
            .nodes
            .iter()
            .find(|node| &node.locator == locator)?;
        let after = self.nodes.iter().find(|node| &node.locator == locator)?;
        Some(Growth {
            before,
            after,
            delta_bytes: i128::from(after.subtree_bytes) - i128::from(before.subtree_bytes),
        })
    }

    /// A conservative list of non-overlapping directories with explicit rebuildable evidence.
    /// This is a review queue, not authorization or an estimate of bytes actually freed.
    pub fn candidates(&self, target_bytes: u64) -> Vec<Candidate<'_>> {
        if !self.snapshot.coverage.complete || target_bytes == 0 {
            return Vec::new();
        }
        let by_node: HashMap<u64, Vec<&EvidenceEdge>> =
            self.evidence.iter().fold(HashMap::new(), |mut map, edge| {
                map.entry(edge.node_id).or_default().push(edge);
                map
            });
        let parents: HashMap<u64, Option<u64>> = self
            .nodes
            .iter()
            .map(|node| (node.id, node.parent_id))
            .collect();
        let blocked: HashSet<u64> = self
            .evidence
            .iter()
            .filter(|edge| {
                matches!(
                    edge.relation,
                    EvidenceRelation::Protected | EvidenceRelation::UsedByProcess
                )
            })
            .map(|edge| edge.node_id)
            .collect();
        let mut eligible: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.kind == crate::NodeKind::Directory && node.subtree_bytes > 0)
            .filter(|node| {
                by_node.get(&node.id).is_some_and(|edges| {
                    edges.iter().any(|edge| {
                        edge.relation == EvidenceRelation::Rebuildable && edge.confidence > 0
                    })
                })
            })
            .collect();
        eligible.sort_by_key(|node| Reverse(node.subtree_bytes));
        let mut selected = HashSet::new();
        let mut result = Vec::new();
        let mut total = 0_u64;
        for node in eligible {
            let mut ancestor = Some(node.id);
            let mut skip = false;
            while let Some(id) = ancestor {
                if blocked.contains(&id) || selected.contains(&id) {
                    skip = true;
                    break;
                }
                ancestor = parents.get(&id).copied().flatten();
            }
            if skip || has_blocked_descendant(node.id, &blocked, &parents) {
                continue;
            }
            selected.insert(node.id);
            total = total.saturating_add(node.subtree_bytes);
            result.push(Candidate {
                node,
                evidence: by_node.get(&node.id).cloned().unwrap_or_default(),
            });
            if total >= target_bytes {
                break;
            }
        }
        result
    }
}

/// The fields a tree view actually renders. The full `DiskNode` carries
/// locators, hints, and identities that a tree never shows; building one
/// from this narrow shape skips all of that (the store's fast read path
/// passes these through without any JSON at all).
#[derive(Clone, Copy, Debug)]
pub struct TreeNode<'a> {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub name: &'a str,
    pub kind: crate::NodeKind,
    pub subtree_bytes: u64,
    pub direct_bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub read_error: bool,
}

/// A depth-bounded tree view of one published revision, in the shape a tree
/// UI consumes: each node carries its aggregate size, its own bytes, and
/// children sorted largest-first. Cutting at the depth bound is reported
/// with `truncated: true` and the child count, so the JSON never claims to
/// have shown more than it did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeView {
    pub root: serde_json::Value,
}

pub fn render_tree(
    graph: &DiskGraph,
    depth: usize,
    min_bytes: u64,
) -> Result<TreeView, TreeRenderError> {
    let rows: Vec<TreeNode<'_>> = graph
        .nodes
        .iter()
        .map(|node| TreeNode {
            id: node.id,
            parent_id: node.parent_id,
            name: node.name.as_str(),
            kind: node.kind,
            subtree_bytes: node.subtree_bytes,
            direct_bytes: node.direct_bytes,
            files: node.files,
            directories: node.directories,
            read_error: node.read_error,
        })
        .collect();
    render_tree_rows(&rows, depth, min_bytes)
}

/// Renders a tree view from narrow node rows (the store's fast read path).
pub fn render_tree_rows(
    rows: &[TreeNode<'_>],
    depth: usize,
    min_bytes: u64,
) -> Result<TreeView, TreeRenderError> {
    use serde_json::json;

    let root = rows
        .iter()
        .find(|node| node.parent_id.is_none())
        .ok_or(TreeRenderError::NoRoot)?;
    let children_of: std::collections::HashMap<u64, Vec<&TreeNode<'_>>> =
        rows.iter()
            .fold(std::collections::HashMap::new(), |mut map, node| {
                if let Some(parent) = node.parent_id {
                    map.entry(parent).or_default().push(node);
                }
                map
            });

    fn render(
        current: &TreeNode<'_>,
        children_of: &std::collections::HashMap<u64, Vec<&TreeNode<'_>>>,
        depth: usize,
        max_depth: usize,
        min_bytes: u64,
    ) -> serde_json::Value {
        let mut value = json!({
            "name": current.name,
            "kind": current.kind,
            "size_bytes": current.subtree_bytes,
            "own_bytes": current.direct_bytes,
            "files": current.files,
            "dirs": current.directories,
        });
        if current.read_error {
            value["read_error"] = json!(true);
        }
        let kids = children_of.get(&current.id).cloned().unwrap_or_default();
        if depth >= max_depth || kids.is_empty() {
            if !kids.is_empty() {
                value["truncated"] = json!(true);
                value["children_count"] = json!(kids.len());
            }
            return value;
        }
        let kept: Vec<_> = kids
            .iter()
            .filter(|kid| kid.subtree_bytes >= min_bytes)
            .cloned()
            .collect();
        let hidden = kids.len() - kept.len();
        let mut ordered = kept;
        ordered.sort_by_key(|kid| (std::cmp::Reverse(kid.subtree_bytes), kid.name));
        value["children"] = json!(
            ordered
                .iter()
                .map(|kid| render(kid, children_of, depth + 1, max_depth, min_bytes))
                .collect::<Vec<_>>()
        );
        if hidden > 0 {
            value["hidden_below_min_bytes"] = json!(hidden);
        }
        value
    }

    Ok(TreeView {
        root: render(root, &children_of, 1, depth, min_bytes),
    })
}

/// Why a tree view could not be rendered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TreeRenderError {
    NoRoot,
}

impl std::fmt::Display for TreeRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoRoot => "the revision has no root node",
        })
    }
}

fn has_blocked_descendant(
    node_id: u64,
    blocked: &HashSet<u64>,
    parents: &HashMap<u64, Option<u64>>,
) -> bool {
    blocked.iter().any(|blocked_id| {
        let mut ancestor = parents.get(blocked_id).copied().flatten();
        while let Some(id) = ancestor {
            if id == node_id {
                return true;
            }
            ancestor = parents.get(&id).copied().flatten();
        }
        false
    })
}

/// One observed difference between two comparable snapshots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Change<'a> {
    Added {
        node: &'a DiskNode,
    },
    Removed {
        locator: &'a ResourceLocator,
        subtree_bytes: u64,
        name: &'a str,
    },
    SizeChanged {
        node: &'a DiskNode,
        previous_bytes: u64,
    },
}

/// Why two snapshots cannot be compared; always reported, never guessed away.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Incompatibility {
    DifferentRoot,
    DifferentVolume,
    UnknownVolume,
    DifferentSettings,
    OutOfOrder,
    IncompleteCoverage,
}

/// Result of comparing two snapshots by exact lossless locator.
pub struct Changes<'a> {
    pub incompatible: Option<Incompatibility>,
    pub changes: Vec<Change<'a>>,
}

impl DiskGraph {
    /// Differences between two snapshots by exact locator. Renames are
    /// intentionally reported as removal + addition, never inferred (Q-04).
    pub fn changes<'a>(&'a self, previous: &'a Self) -> Changes<'a> {
        let incompatible = if self.snapshot.root != previous.snapshot.root {
            Some(Incompatibility::DifferentRoot)
        } else if self.snapshot.volume_id.is_none() || previous.snapshot.volume_id.is_none() {
            Some(Incompatibility::UnknownVolume)
        } else if self.snapshot.volume_id != previous.snapshot.volume_id {
            Some(Incompatibility::DifferentVolume)
        } else if self.snapshot.settings != previous.snapshot.settings {
            Some(Incompatibility::DifferentSettings)
        } else if self.snapshot.captured_at_unix_ms < previous.snapshot.captured_at_unix_ms {
            Some(Incompatibility::OutOfOrder)
        } else if !self.snapshot.coverage.complete || !previous.snapshot.coverage.complete {
            Some(Incompatibility::IncompleteCoverage)
        } else {
            None
        };
        let mut changes = Vec::new();
        if incompatible.is_some() {
            return Changes {
                incompatible,
                changes,
            };
        }
        let after: HashMap<&ResourceLocator, &DiskNode> = self
            .nodes
            .iter()
            .map(|node| (&node.locator, node))
            .collect();
        let before: HashMap<&ResourceLocator, &DiskNode> = previous
            .nodes
            .iter()
            .map(|node| (&node.locator, node))
            .collect();
        for (locator, node) in &after {
            match before.get(locator) {
                Some(previous_node) => {
                    if previous_node.subtree_bytes != node.subtree_bytes {
                        changes.push(Change::SizeChanged {
                            node,
                            previous_bytes: previous_node.subtree_bytes,
                        });
                    }
                }
                None => changes.push(Change::Added { node }),
            }
        }
        for (locator, node) in &before {
            if !after.contains_key(locator) {
                changes.push(Change::Removed {
                    locator,
                    subtree_bytes: node.subtree_bytes,
                    name: &node.name,
                });
            }
        }
        changes.sort_by_key(|change| match change {
            Change::Added { node } | Change::SizeChanged { node, .. } => node.name.clone(),
            Change::Removed { name, .. } => (*name).to_owned(),
        });
        Changes {
            incompatible: None,
            changes,
        }
    }
}

/// A size filter for listing queries; `Unknown` keeps unscanned or
/// unreported sizes visible instead of silently dropping them (Q-06).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeFilter {
    /// Only nodes with a reported subtree size above the threshold.
    AtLeast(u64),
    /// Only nodes with no reported size (unscanned, denied, or provider-limited).
    UnknownOnly,
}

/// How a child listing ended, including honest coverage reporting.
pub struct ChildListing<'a> {
    pub items: Vec<&'a DiskNode>,
    pub next_offset: Option<usize>,
    /// Nodes whose size could not be reported, kept out of the ordering.
    pub unknown_count: usize,
    pub truncated: bool,
}

impl DiskGraph {
    /// Direct children with a size filter, stable ordering, and paging.
    /// Unknown sizes are counted separately and never mixed into byte order.
    pub fn children_filtered<'a>(
        &'a self,
        parent_id: u64,
        filter: Option<SizeFilter>,
        offset: usize,
        limit: usize,
    ) -> ChildListing<'a> {
        let mut known: Vec<&DiskNode> = Vec::new();
        let mut unknown_count = 0usize;
        for node in &self.nodes {
            if node.parent_id != Some(parent_id) {
                continue;
            }
            // A node that is a directory but unreadable, or a provider resource
            // with no size, is "unknown" rather than zero bytes.
            let sized = !node.read_error && node.size_known;
            match (&filter, sized) {
                (Some(SizeFilter::AtLeast(minimum)), true) if node.subtree_bytes >= *minimum => {
                    known.push(node)
                }
                (Some(SizeFilter::AtLeast(_)), true) => {}
                (Some(SizeFilter::AtLeast(_)), false) => unknown_count += 1,
                (Some(SizeFilter::UnknownOnly), false) => known.push(node),
                (Some(SizeFilter::UnknownOnly), true) => {}
                (None, true) => known.push(node),
                (None, false) => unknown_count += 1,
            }
        }
        known.sort_by(|a, b| {
            b.subtree_bytes
                .cmp(&a.subtree_bytes)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
        let total = known.len();
        let end = offset.saturating_add(limit).min(total);
        let items = known
            .get(offset..end)
            .map(|slice| slice.to_vec())
            .unwrap_or_default();
        ChildListing {
            items,
            next_offset: (end < total).then_some(end),
            unknown_count,
            truncated: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, Incompatibility, SizeFilter, TreeRenderError, render_tree};
    use crate::{
        DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, NodeKind,
        ResourceLocator, ScanCoverage, ScanSettings,
    };

    fn node(id: u64, parent_id: Option<u64>, name: &str, bytes: u64) -> DiskNode {
        DiskNode {
            id,
            parent_id,
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

    fn graph(bytes: u64) -> DiskGraph {
        DiskGraph {
            snapshot: DiskSnapshot {
                id: "snapshot".into(),
                root: ResourceLocator::NativePath("/tmp".into()),
                volume_id: Some("test-volume".into()),
                captured_at_unix_ms: bytes,
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
                node(1, None, "root", bytes),
                node(2, Some(1), "cache", bytes),
                node(3, Some(2), "protected", 10),
            ],
            evidence: vec![EvidenceEdge {
                node_id: 2,
                relation: EvidenceRelation::Rebuildable,
                subject: "build output".into(),
                source: "test".into(),
                observed_at_unix_ms: 0,
                confidence: 80,
            }],
        }
    }

    #[test]
    fn queries_are_bounded_and_growth_requires_complete_compatible_scans() {
        let before = graph(100);
        let after = graph(150);
        assert_eq!(after.top(1, 1)[0].id, 2);
        assert_eq!(after.children(1, 0, 1).items.len(), 1);
        assert_eq!(after.explain(2).unwrap().evidence.len(), 1);
        assert_eq!(
            after
                .growth(&before, &ResourceLocator::NativePath("/tmp/cache".into()))
                .unwrap()
                .delta_bytes,
            50
        );
        let mut partial = before;
        partial.snapshot.coverage.complete = false;
        assert!(after.growth(&partial, &after.nodes[1].locator).is_none());
    }

    #[test]
    fn candidates_never_include_a_protected_descendant() {
        let mut graph = graph(150);
        assert_eq!(graph.candidates(100).len(), 1);
        graph.evidence.push(EvidenceEdge {
            node_id: 3,
            relation: EvidenceRelation::Protected,
            subject: "active data".into(),
            source: "test".into(),
            observed_at_unix_ms: 0,
            confidence: 100,
        });
        assert!(graph.candidates(100).is_empty());
        graph.snapshot.coverage.complete = false;
        graph.evidence.pop();
        assert!(graph.candidates(100).is_empty());
    }

    /// root → cache(large) → {beta, alpha}; cache carries a read error.
    fn tree_fixture() -> DiskGraph {
        let mut graph = graph(100);
        graph.nodes.retain(|node| node.id != 3); // drop the protected child
        let mut alpha = node(4, Some(2), "alpha", 10);
        alpha.kind = crate::NodeKind::Directory;
        alpha.directories = 1;
        let mut beta = node(5, Some(2), "beta", 20);
        beta.kind = crate::NodeKind::Directory;
        beta.directories = 1;
        graph.nodes.push(alpha);
        graph.nodes.push(beta);
        graph
    }

    #[test]
    fn tree_view_sorts_children_largest_first() {
        let graph = tree_fixture();
        let view = render_tree(&graph, 3, 0).unwrap();
        let cache = &view.root["children"][0];
        let names: Vec<&str> = cache["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kid| kid["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["beta", "alpha"], "largest first");
        assert_eq!(cache["truncated"], serde_json::Value::Null);
    }

    #[test]
    fn tree_view_marks_truncation_and_hides_below_min_bytes() {
        let graph = tree_fixture();

        // Depth 2 cuts inside cache: the cut is declared, never silent.
        let view = render_tree(&graph, 2, 0).unwrap();
        let cache = &view.root["children"][0];
        assert_eq!(cache["truncated"], true);
        assert_eq!(cache["children_count"], 2);
        assert!(cache.get("children").is_none());

        // A min-bytes floor hides the small child and says how many.
        let view = render_tree(&graph, 3, 15).unwrap();
        let cache = &view.root["children"][0];
        let names: Vec<&str> = cache["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kid| kid["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["beta"]);
        assert_eq!(cache["hidden_below_min_bytes"], 1);
    }

    #[test]
    fn tree_view_carries_read_errors_and_requires_a_root() {
        let mut graph = tree_fixture();
        graph.nodes[1].read_error = true; // cache
        let view = render_tree(&graph, 3, 0).unwrap();
        assert_eq!(view.root["children"][0]["read_error"], true);

        let mut empty = graph;
        empty.nodes.clear();
        assert!(matches!(
            render_tree(&empty, 2, 0),
            Err(TreeRenderError::NoRoot)
        ));
    }

    #[test]
    fn snapshot_json_round_trip_preserves_locators() {
        let graph = graph(100);
        let encoded = serde_json::to_string(&graph).unwrap();
        let decoded: DiskGraph = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, graph);
    }

    #[test]
    fn changes_report_additions_removals_and_size_shifts_without_renames() {
        let before = graph(100);
        let mut after = graph(150);
        let mut extra = node(4, Some(1), "added", 25);
        extra.locator = ResourceLocator::NativePath("/tmp/added".into());
        after.nodes.push(extra);

        let report = after.changes(&before);
        assert!(report.incompatible.is_none());
        assert!(
            report
                .changes
                .iter()
                .any(|change| matches!(change, Change::Added { node } if node.name == "added"))
        );
        assert!(report.changes.iter().any(|change| matches!(
            change,
            Change::SizeChanged {
                previous_bytes: 100,
                ..
            }
        )));

        let mut pruned = graph(150);
        pruned.nodes.retain(|node| node.id != 3);
        let report = pruned.changes(&before);
        assert!(report.changes.iter().any(|change| matches!(
            change,
            Change::Removed { name, .. } if *name == "protected"
        )));
    }

    #[test]
    fn changes_refuse_incompatible_snapshots_with_reasons() {
        let before = graph(100);
        let mut after = graph(100);
        after.snapshot.volume_id = Some("other-volume".into());
        assert!(matches!(
            after.changes(&before).incompatible,
            Some(Incompatibility::DifferentVolume)
        ));
        let mut partial = graph(100);
        partial.snapshot.coverage.complete = false;
        assert!(matches!(
            partial.changes(&before).incompatible,
            Some(Incompatibility::IncompleteCoverage)
        ));
        let mut other_root = graph(100);
        other_root.snapshot.root = ResourceLocator::NativePath("/elsewhere".into());
        other_root.nodes[0].locator = ResourceLocator::NativePath("/elsewhere".into());
        assert!(matches!(
            other_root.changes(&before).incompatible,
            Some(Incompatibility::DifferentRoot)
        ));
    }

    #[test]
    fn children_filtering_keeps_unknown_sizes_visible_and_out_of_byte_order() {
        let mut graph = graph(150);
        // A node whose size could not be reported.
        let mut unknown = node(5, Some(1), "denied", 0);
        unknown.size_known = false;
        unknown.read_error = true;
        graph.nodes.push(unknown);

        // Default listing sorts by bytes and counts the unknown separately.
        // The root owns `cache` and the unsized `denied`; `protected` sits
        // under `cache`.
        let listing = graph.children_filtered(1, None, 0, 10);
        assert_eq!(listing.items.len(), 1);
        assert_eq!(listing.items[0].name, "cache");
        assert_eq!(listing.unknown_count, 1);
        assert!(listing.items.iter().all(|node| node.size_known));

        // Unknown-only surfaces exactly the unsized node.
        let unknown_only = graph.children_filtered(1, Some(SizeFilter::UnknownOnly), 0, 10);
        assert_eq!(unknown_only.items.len(), 1);
        assert_eq!(unknown_only.items[0].name, "denied");

        // A size threshold never admits an unknown size as zero.
        let threshold = graph.children_filtered(1, Some(SizeFilter::AtLeast(100)), 0, 10);
        assert!(threshold.items.iter().all(|node| node.subtree_bytes >= 100));
        assert_eq!(threshold.unknown_count, 1);

        // Paging over the two direct children is stable and resumable.
        let unsized_first = graph.children_filtered(1, Some(SizeFilter::UnknownOnly), 0, 1);
        let unsized_all = graph.children_filtered(1, Some(SizeFilter::UnknownOnly), 0, 10);
        assert_eq!(unsized_first.items.len(), 1);
        assert_eq!(unsized_all.next_offset, None);
        // A zero threshold admits every node with a known size.
        let all_known = graph.children_filtered(1, Some(SizeFilter::AtLeast(0)), 0, 10);
        assert_eq!(all_known.items.len(), 1);
        assert_eq!(all_known.unknown_count, 1);
    }
}
