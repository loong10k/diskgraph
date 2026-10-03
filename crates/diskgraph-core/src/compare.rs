//! Directory comparison: what two trees have that the other does not.
//!
//! This is a different question from `query::changes`. That one asks what
//! happened to *one* directory between two points in time, and refuses to
//! answer when the roots differ. This one takes *two* roots - a release
//! build and the working copy, two machines, a backup - and answers for each
//! path whether it is only on the left, only on the right, different, or the
//! same. So it lives beside `changes` rather than inside it: folding the two
//! together would have meant loosening the comparability contract that
//! `changes` keeps on purpose.
//!
//! Every verdict says which test produced it. A path reported identical
//! because its size and timestamp match is not the same claim as one reported
//! identical because its bytes were compared, and a caller that has to
//! decide whether to re-copy a file needs to know which one it got.

use crate::{DiskNode, ResourceLocator, observed_node_size};
use std::collections::HashMap;

mod comparison;
mod different_reason;
mod evidence;
mod summary;
mod verdict;

pub use comparison::Comparison;
pub use different_reason::DifferentReason;
pub use evidence::Evidence;
pub use summary::Summary;
pub use verdict::Verdict;

/// Compares two trees that share a root prefix, entry by entry.
///
/// Both sides are indexed by their path relative to their own root, so a file
/// at `src/lib.rs` on one side is matched against `src/lib.rs` on the other
/// and a directory that only exists on one side is reported as such instead of
/// being compared entry by entry against nothing.
/// 允许不同根的相对路径对齐；只比较已观察元数据，不要求两个文件身份相同。
/// 参数：left_root/left 与 right_root/right 为两侧根及节点，tolerance_seconds 为时间容忍秒数。
/// 返回：逐路径判定及汇总，单侧存在按对应侧报告，未知尺寸单独计数。
pub fn compare<'a>(
    left_root: &DiskNode,
    left: &'a [DiskNode],
    right_root: &DiskNode,
    right: &'a [DiskNode],
    tolerance_seconds: i64,
) -> (Vec<Comparison<'a>>, Summary) {
    let left_index = by_relative_path(left_root, left);
    let right_index = by_relative_path(right_root, right);
    let mut paths: Vec<&String> = left_index.keys().chain(right_index.keys()).collect();
    paths.sort_unstable();
    paths.dedup();

    let mut out = Vec::new();
    let mut summary = Summary::default();
    for path in paths {
        let on_left = left_index.get(path).copied();
        let on_right = right_index.get(path).copied();
        let verdict = match (on_left, on_right) {
            (Some(_), None) => {
                summary.left_only += 1;
                Verdict::LeftOnly
            }
            (None, Some(_)) => {
                summary.right_only += 1;
                Verdict::RightOnly
            }
            (Some(left_node), Some(right_node)) => {
                let verdict = compare_entry(left_node, right_node, tolerance_seconds);
                match &verdict {
                    Verdict::Same { .. } => summary.same += 1,
                    Verdict::Different { reason } => {
                        summary.different += 1;
                        if *reason == DifferentReason::UnknownSize {
                            summary.unknown += 1;
                        }
                    }
                    _ => {}
                }
                verdict
            }
            (None, None) => continue,
        };
        out.push(Comparison {
            path: (*path).clone(),
            verdict,
            left: on_left,
            right: on_right,
        });
    }
    (out, summary)
}

/// 比较两侧同一路径的单个节点，返回元数据证据支持的判定。
/// 参数：left/right 为两侧节点，tolerance_seconds 为时间差容忍秒数；缺失时间不推断差异。
/// 返回：未知优先于类型差异，然后检查目录聚合、大小及时间的元数据判定。
pub fn compare_entry(left: &DiskNode, right: &DiskNode, tolerance_seconds: i64) -> Verdict {
    // 未观察到的尺寸优先于类型和目录聚合，不能用占位数字推断差异或相同。
    if observed_node_size(left).is_none() || observed_node_size(right).is_none() {
        return Verdict::Different {
            reason: DifferentReason::UnknownSize,
        };
    }
    if left.kind != right.kind {
        return Verdict::Different {
            reason: DifferentReason::Path,
        };
    }
    // A directory's own size and timestamp say nothing about whether its
    // contents match - they move whenever anything under them is touched, and
    // a restored tree keeps its directory timestamps. A directory is compared
    // by what it holds, which the caller reaches through its children; here it
    // is only equal when the counts agree.
    if is_directory(left) && is_directory(right) {
        return if left.files == right.files
            && left.directories == right.directories
            && left.subtree_bytes == right.subtree_bytes
        {
            Verdict::Same {
                evidence: Evidence::Metadata,
            }
        } else {
            Verdict::Different {
                reason: DifferentReason::Contents,
            }
        };
    }
    if left.subtree_bytes != right.subtree_bytes {
        return Verdict::Different {
            reason: DifferentReason::Size,
        };
    }
    // The same bytes at a different time is a real difference in a tree that
    // gets copied around, but a weaker one than content: re-copying is
    // rarely worth it, so the reason is named rather than folded into Same.
    let left_time = left.modified_unix_seconds;
    let right_time = right.modified_unix_seconds;
    let times_differ = match (left_time, right_time) {
        (Some(left_time), Some(right_time)) => {
            (i128::from(left_time) - i128::from(right_time)).abs()
                > i128::from(tolerance_seconds.max(0))
        }
        // One side has no timestamp and the other does: not evidence of a
        // difference, so the entries stay the same.
        _ => false,
    };
    if times_differ {
        return Verdict::Different {
            reason: DifferentReason::Timestamp,
        };
    }
    Verdict::Same {
        evidence: Evidence::Metadata,
    }
}

fn is_directory(node: &DiskNode) -> bool {
    node.kind == crate::model::NodeKind::Directory
}

/// Indexes a side by path relative to its own root, which is what makes two
/// different roots comparable at all.
fn by_relative_path<'a>(root: &DiskNode, nodes: &'a [DiskNode]) -> HashMap<String, &'a DiskNode> {
    let prefix = locator_path(&root.locator);
    nodes
        .iter()
        .filter(|node| node.parent_id.is_some())
        .map(|node| {
            let path = locator_path(&node.locator);
            // Stripping the root leaves a leading separator, and a path
            // written "/same.bin" would never match one written "same.bin":
            // the relative form is the join key for the whole comparison.
            let relative = path
                .strip_prefix(&prefix)
                .map(|rest| rest.trim_start_matches('/').to_owned())
                .unwrap_or_else(|| node.name.clone());
            (relative, node)
        })
        .collect()
}

fn locator_path(locator: &ResourceLocator) -> String {
    match locator {
        ResourceLocator::NativePath(path) => path.clone(),
        ResourceLocator::DocumentUri(uri) => uri.clone(),
    }
}

/// Rolls a directory's own verdict up from the verdicts of what it holds.
///
/// A directory whose only child changed is a directory that changed, and a
/// caller asking "did this tree change" wants that said about the top of the
/// tree rather than only about leaves.
/// 子项未知优先于普通差异，单侧子项表示目录内容变化，不能声称其余子项相同。
/// 参数：verdicts 为已比较的子项判定；返回：目录的元数据汇总判定。
pub fn roll_up(verdicts: &[Verdict]) -> Verdict {
    let mut differing = 0_u64;
    let mut on_left_only = 0_u64;
    let mut on_right_only = 0_u64;
    let mut unknown = 0_u64;
    for verdict in verdicts {
        match verdict {
            Verdict::LeftOnly => on_left_only += 1,
            Verdict::RightOnly => on_right_only += 1,
            Verdict::Same { .. } => {}
            Verdict::Different { reason } => {
                differing += 1;
                if *reason == DifferentReason::UnknownSize {
                    unknown += 1;
                }
            }
        }
    }
    if differing == 0 && on_left_only == 0 && on_right_only == 0 {
        return Verdict::Same {
            evidence: Evidence::Metadata,
        };
    }
    // A child present on one side only is a difference in what the directory
    // holds, not in the directory's path: `Path` is for an entry reached two
    // ways, and nothing in a roll-up can produce that. An unreadable subtree
    // is not a claim that the rest matches either, so it wins outright.
    let reason = if unknown > 0 {
        DifferentReason::UnknownSize
    } else {
        DifferentReason::Contents
    };
    Verdict::Different { reason }
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
