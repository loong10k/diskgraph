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
            || self.snapshot.settings != previous.snapshot.settings
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

#[cfg(test)]
mod tests {
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
                volume_id: None,
                captured_at_unix_ms: 0,
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

    #[test]
    fn snapshot_json_round_trip_preserves_locators() {
        let graph = graph(100);
        let encoded = serde_json::to_string(&graph).unwrap();
        let decoded: DiskGraph = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, graph);
    }
}
