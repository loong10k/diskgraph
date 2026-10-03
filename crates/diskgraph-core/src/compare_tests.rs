//! 既有跨根比较行为回归；来源：DiskGraph 原生 Rust compare 测试。
use super::{DifferentReason, Evidence, Summary, Verdict, compare, roll_up};
use crate::model::{FileIdentity, NodeKind};
use crate::{DiskNode, ResourceLocator};

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
