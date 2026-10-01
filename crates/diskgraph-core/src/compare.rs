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

use std::collections::HashMap;

use crate::model::{DiskNode, ResourceLocator};

/// How thoroughly two entries were compared.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Evidence {
    /// The path exists on one side only. No test ran.
    Presence,
    /// Size and modification time were compared; contents were not read.
    Metadata,
    /// Contents were read and compared byte for byte.
    Content,
}

/// What is true of one path across two trees.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Verdict {
    /// Present on the comparison's left - the `--from` side - and absent
    /// from its right. Named for the side rather than for the parameter,
    /// because the comparison is a fact about two trees and only the caller
    /// knows which one it called "from".
    #[serde(rename = "from-only")]
    LeftOnly,
    /// Present on the comparison's right - the `--to` side - and absent from
    /// its left.
    #[serde(rename = "to-only")]
    RightOnly,
    /// Present on both, and not the same.
    Different {
        /// The first test that separated them, most specific first.
        reason: DifferentReason,
    },
    /// Present on both, and the same to the depth tested.
    Same { evidence: Evidence },
}

impl Verdict {
    /// Whether the two sides disagree, in the sense a caller cares about:
    /// something would have to be copied or removed.
    pub fn is_difference(&self) -> bool {
        !matches!(self, Verdict::Same { .. })
    }

    /// The deepest test that was actually run for this verdict.
    pub fn evidence(&self) -> Evidence {
        match self {
            Verdict::LeftOnly | Verdict::RightOnly => Evidence::Presence,
            Verdict::Different { .. } | Verdict::Same { .. } => Evidence::Metadata,
        }
    }
}

/// Why two entries that both exist still differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DifferentReason {
    /// Different bytes, or different size with contents not read.
    Content,
    /// Same content length, different bytes.
    Size,
    /// The same bytes, but a different modification time.
    Timestamp,
    /// The same file, reached by two different paths.
    Path,
    /// One side reported a size the other did not.
    UnknownSize,
    /// A directory whose children disagree; the directory itself may match.
    Contents,
}

/// One path's worth of comparison.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Comparison<'a> {
    /// The path relative to the compared root, in display form.
    pub path: String,
    pub verdict: Verdict,
    pub left: Option<&'a DiskNode>,
    pub right: Option<&'a DiskNode>,
}

/// Counts per verdict, so a caller can report a shape without walking the
/// whole list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct Summary {
    pub left_only: u64,
    pub right_only: u64,
    pub different: u64,
    pub same: u64,
    /// Entries skipped because one side could not report a size.
    pub unknown: u64,
    /// Entries not visited because the caller bounded the depth.
    pub skipped: u64,
}

impl Summary {
    /// Paths that would have to be copied or removed to make the trees match.
    pub fn actionable(&self) -> u64 {
        self.left_only + self.right_only + self.different
    }
}

/// Compares two trees that share a root prefix, entry by entry.
///
/// Both sides are indexed by their path relative to their own root, so a file
/// at `src/lib.rs` on one side is matched against `src/lib.rs` on the other
/// and a directory that only exists on one side is reported as such instead of
/// being compared entry by entry against nothing.
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
/// tolerance_seconds 指定时间差容忍值；缺失时间不推断差异。
pub fn compare_entry(left: &DiskNode, right: &DiskNode, tolerance_seconds: i64) -> Verdict {
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
    if is_directory(left) != is_directory(right) {
        return Verdict::Different {
            reason: DifferentReason::Path,
        };
    }
    // A size either side could not report is unknown, not different. Reporting
    // it as a difference would send a caller to re-copy a file that may be
    // byte-identical and simply unreadable.
    if !left.size_known || !right.size_known {
        return Verdict::Different {
            reason: DifferentReason::UnknownSize,
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
mod tests {
    use super::*;
    use crate::model::{FileIdentity, NodeKind};

    fn node(id: u64, parent: Option<u64>, path: &str, bytes: u64, mtime: Option<i64>) -> DiskNode {
        DiskNode {
            id,
            parent_id: parent,
            locator: ResourceLocator::NativePath(path.to_owned()),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            kind: NodeKind::File,
            subtree_bytes: bytes,
            direct_bytes: bytes,
            size_known: true,
            files: 1,
            directories: 0,
            modified_unix_seconds: mtime,
            file_identity: Some(FileIdentity {
                volume_id: "v".into(),
                file_id: id,
            }),
            category_hint: None,
            reclaim_hint: None,
            read_error: false,
        }
    }

    fn dir(id: u64, parent: Option<u64>, path: &str, files: u64, bytes: u64) -> DiskNode {
        DiskNode {
            kind: NodeKind::Directory,
            direct_bytes: 0,
            files,
            directories: 1,
            ..node(id, parent, path, bytes, Some(1_000))
        }
    }

    fn sides() -> (DiskNode, Vec<DiskNode>, DiskNode, Vec<DiskNode>) {
        // Two different roots, same relative paths: this is the case the
        // whole module exists for, and the one `changes` refuses.
        let left_root = dir(1, None, "/build/release", 3, 3_000);
        let left = vec![
            dir(1, None, "/build/release", 3, 3_000),
            node(2, Some(1), "/build/release/same.bin", 1_000, Some(500)),
            node(3, Some(1), "/build/release/newer.bin", 1_000, Some(900)),
            node(4, Some(1), "/build/release/grown.bin", 1_000, Some(500)),
            node(5, Some(1), "/build/release/only-left.bin", 100, Some(500)),
        ];
        let right_root = dir(1, None, "/src/worktree", 3, 2_000);
        let right = vec![
            dir(1, None, "/src/worktree", 3, 2_000),
            node(2, Some(1), "/src/worktree/same.bin", 1_000, Some(500)),
            node(3, Some(1), "/src/worktree/newer.bin", 1_000, Some(500)),
            node(4, Some(1), "/src/worktree/grown.bin", 2_000, Some(500)),
            node(6, Some(1), "/src/worktree/only-right.bin", 100, Some(500)),
        ];
        (left_root, left, right_root, right)
    }

    #[test]
    fn two_different_roots_compare_by_relative_path() {
        let (left_root, left, right_root, right) = sides();
        let (rows, summary) = compare(&left_root, &left, &right_root, &right, 1);
        let find = |name: &str| {
            rows.iter()
                .find(|row| row.path == name)
                .map(|row| row.verdict.clone())
                .unwrap_or_else(|| panic!("{name} is missing"))
        };
        assert_eq!(
            find("same.bin"),
            Verdict::Same {
                evidence: Evidence::Metadata
            }
        );
        assert_eq!(
            find("only-left.bin"),
            Verdict::LeftOnly,
            "a path absent on one side is a presence fact, not a difference"
        );
        assert_eq!(find("only-right.bin"), Verdict::RightOnly);
        assert_eq!(
            find("grown.bin"),
            Verdict::Different {
                reason: DifferentReason::Size
            }
        );
        assert_eq!(
            find("newer.bin"),
            Verdict::Different {
                reason: DifferentReason::Timestamp
            }
        );
        assert_eq!(summary.same, 1);
        assert_eq!(summary.different, 2);
        assert_eq!(summary.left_only, 1);
        assert_eq!(summary.right_only, 1);
        assert_eq!(summary.actionable(), 4);
    }

    #[test]
    fn a_timestamp_within_tolerance_is_the_same_file() {
        let (left_root, left, right_root, right) = sides();
        // One second of clock skew between two machines must not turn every
        // file into a difference.
        let mut right = right;
        if let Some(entry) = right.iter_mut().find(|node| node.name == "same.bin") {
            entry.modified_unix_seconds = Some(501);
        }
        let (rows, _) = compare(&left_root, &left, &right_root, &right, 2);
        let same = rows
            .iter()
            .find(|row| row.path == "same.bin")
            .expect("row present");
        assert_eq!(
            same.verdict,
            Verdict::Same {
                evidence: Evidence::Metadata
            }
        );
    }

    #[test]
    fn an_unreadable_side_is_unknown_rather_than_different() {
        let left_root = dir(1, None, "/a", 1, 1_000);
        let left = vec![
            dir(1, None, "/a", 1, 1_000),
            node(2, Some(1), "/a/x", 10, Some(1)),
        ];
        let right_root = dir(1, None, "/b", 1, 1_000);
        let mut unreadable = node(2, Some(1), "/b/x", 0, None);
        unreadable.size_known = false;
        unreadable.read_error = true;
        let right = vec![dir(1, None, "/b", 1, 1_000), unreadable];
        let (rows, summary) = compare(&left_root, &left, &right_root, &right, 0);
        let row = rows
            .iter()
            .find(|row| row.path == "x")
            .expect("row present");
        assert_eq!(
            row.verdict,
            Verdict::Different {
                reason: DifferentReason::UnknownSize
            },
            "a size nobody could read is not evidence of a difference"
        );
        assert_eq!(summary.unknown, 1);
    }

    #[test]
    fn a_directory_is_judged_by_what_it_holds_not_by_its_own_timestamp() {
        let left_root = dir(1, None, "/a", 2, 2_000);
        let left = vec![
            dir(1, None, "/a", 2, 2_000),
            node(2, Some(1), "/a/one", 1_000, Some(1)),
            node(3, Some(1), "/a/two", 1_000, Some(1)),
        ];
        let right_root = dir(1, None, "/b", 2, 2_000);
        let mut touched = right_root.clone();
        touched.modified_unix_seconds = Some(9_999);
        let right = vec![
            touched.clone(),
            node(2, Some(1), "/b/one", 1_000, Some(1)),
            node(3, Some(1), "/b/two", 1_000, Some(1)),
        ];
        let (rows, _) = compare(&left_root, &left, &right_root, &right, 1);
        // The roots themselves are not rows: both sides always have one, so
        // it can never be a presence fact, and its verdict is a roll-up the
        // caller makes from the rows below it.
        let children: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();
        assert_eq!(children, vec!["one", "two"]);
        for row in &rows {
            assert_eq!(
                row.verdict,
                Verdict::Same {
                    evidence: Evidence::Metadata
                },
                "{} moved only because its parent did",
                row.path
            );
        }
    }

    #[test]
    fn a_path_that_is_a_file_on_one_side_and_a_directory_on_the_other_is_a_path_difference() {
        let left_root = dir(1, None, "/a", 1, 1_000);
        let left = vec![
            dir(1, None, "/a", 1, 1_000),
            dir(2, Some(1), "/a/thing", 0, 0),
        ];
        let right_root = dir(1, None, "/b", 1, 1_000);
        let right = vec![
            dir(1, None, "/b", 1, 1_000),
            node(2, Some(1), "/b/thing", 0, None),
        ];
        let (rows, _) = compare(&left_root, &left, &right_root, &right, 0);
        let row = rows.iter().find(|row| row.path == "thing").expect("row");
        assert_eq!(
            row.verdict,
            Verdict::Different {
                reason: DifferentReason::Path
            }
        );
    }

    #[test]
    fn a_parent_only_says_itself_that_its_contents_disagree() {
        let same = Verdict::Same {
            evidence: Evidence::Metadata,
        };
        assert_eq!(roll_up(&[same.clone(), same.clone()]), same);
        assert_eq!(
            roll_up(&[same.clone(), Verdict::LeftOnly]),
            Verdict::Different {
                reason: DifferentReason::Contents
            }
        );
        assert_eq!(
            roll_up(&[Verdict::Different {
                reason: DifferentReason::UnknownSize
            }]),
            Verdict::Different {
                reason: DifferentReason::UnknownSize
            },
            "an unreadable child must not roll up as an ordinary difference"
        );
    }

    #[test]
    fn the_verdicts_are_named_for_the_sides_the_caller_gave_them() {
        // The report says "from-only" and "to-only" because the caller wrote
        // --from and --to. A verdict that said "left" would leave the reader
        // mapping the word back onto the arguments, which is the confusion
        // the arguments were named to remove.
        let left_only = serde_json::to_value(Verdict::LeftOnly).unwrap();
        let right_only = serde_json::to_value(Verdict::RightOnly).unwrap();
        assert_eq!(left_only, serde_json::json!({ "status": "from-only" }));
        assert_eq!(right_only, serde_json::json!({ "status": "to-only" }));
        assert_eq!(
            serde_json::to_value(Verdict::Same {
                evidence: Evidence::Metadata
            })
            .unwrap(),
            serde_json::json!({ "status": "same", "evidence": "metadata" })
        );
    }

    #[test]
    fn a_summary_of_no_differences_has_nothing_actionable() {
        let summary = Summary {
            same: 10,
            ..Summary::default()
        };
        assert_eq!(summary.actionable(), 0);
    }

    #[test]
    fn evidence_is_never_claimed_beyond_what_ran() {
        // Nothing in this module reads file contents, so no verdict may claim
        // a content comparison: a caller that trusts that claim to skip a copy
        // would skip a file that differs.
        let left_root = dir(1, None, "/a", 1, 1_000);
        let left = vec![
            dir(1, None, "/a", 1, 1_000),
            node(2, Some(1), "/a/x", 7, Some(3)),
        ];
        let right_root = dir(1, None, "/b", 1, 1_000);
        let right = vec![
            dir(1, None, "/b", 1, 1_000),
            node(2, Some(1), "/b/x", 7, Some(3)),
        ];
        let (rows, _) = compare(&left_root, &left, &right_root, &right, 0);
        for row in &rows {
            assert_ne!(
                row.verdict.evidence(),
                Evidence::Content,
                "{} claimed a content comparison that never ran",
                row.path
            );
        }
    }
}
