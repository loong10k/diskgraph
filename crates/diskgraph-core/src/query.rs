//! 快照查询与历史尺寸资格；来源：DiskGraph 原生 Rust query / D33。

use crate::{
    DiskGraph, DiskNode, EvidenceEdge, EvidenceRelation, ResourceLocator, comparable_growth_delta,
};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

mod candidate;
mod change;
mod changes;
mod child_listing;
mod growth;
mod incompatibility;
mod node_explanation;
mod page;
mod size_filter;
mod tree_node;
mod tree_render_error;
mod tree_view;

pub use candidate::Candidate;
pub use change::Change;
pub use changes::Changes;
pub use child_listing::ChildListing;
pub use growth::Growth;
pub use incompatibility::Incompatibility;
pub use node_explanation::NodeExplanation;
pub use page::Page;
pub use size_filter::SizeFilter;
pub use tree_node::TreeNode;
pub use tree_render_error::TreeRenderError;
pub use tree_view::{TreeView, render_tree, render_tree_rows};

impl DiskGraph {
    /// Largest immediate children of a node; this never returns a delete plan.
    /// 参数：parent_id 为父节点，limit 为最多返回项数；返回：按大小和名称排序的直接子项，不生成删除计划。
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
    /// 参数：parent_id 为父节点，offset/limit 为分页范围；返回：当前页及后续偏移。
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
    /// 参数：node_id 为快照内节点；返回：节点及已记录证据，节点缺失为 None，不生成证据断言。
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
    /// 参数：previous 为旧快照，locator 为不变定位；返回：兼容、同类型且尺寸已知可读时的差值，否则 None，不推断重命名。
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
            delta_bytes: comparable_growth_delta(before, after)?,
        })
    }

    /// A conservative list of non-overlapping directories with explicit rebuildable evidence.
    /// This is a review queue, not authorization or an estimate of bytes actually freed.
    /// 参数：target_bytes 为期望审阅字节数；返回：互不重叠且有重建证据的目录，不授予文件操作权限。
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
            .filter(|node| {
                node.kind == crate::NodeKind::Directory
                    && node.subtree_bytes > 0
                    && node.size_known
                    && !node.read_error
            })
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

impl DiskGraph {
    /// Differences between two snapshots by exact locator. Renames are
    /// intentionally reported as removal + addition, never inferred (Q-04).
    /// 参数：previous 为旧快照；返回：按精确定位匹配的变化或不可比原因，重命名只报告新增/移除，未知尺寸及类型替换不计尺寸变化。
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
                    if comparable_growth_delta(previous_node, node).is_some_and(|delta| delta != 0)
                    {
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

impl DiskGraph {
    /// Direct children with a size filter, stable ordering, and paging.
    /// Unknown sizes are counted separately and never mixed into byte order.
    /// 参数：parent_id 为父节点，filter 为尺寸条件，offset/limit 为页范围；返回：稳定排序页及未知计数，未知项不混入大小排序。
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
#[path = "query_tests.rs"]
mod tests;
