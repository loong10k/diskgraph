//! Turning a comparison into a list of things a sync would do.
//!
//! This produces a plan and nothing else. It opens no file, writes no file,
//! and has no way to be told to go ahead: a plan is what a person reads
//! before deciding, and an agent that can turn "mirror this directory" into a
//! deletion without showing anyone is exactly the tool nobody can run on a
//! machine they care about.
//!
//! The five methods are the ones a two-directory sync has always offered, and
//! they differ only in which way bytes travel and whether anything is removed.
//! The removals are the whole reason this is a separate module: `Mirror` is
//! the only method that deletes, and its deletions are listed as first-class
//! entries in the plan rather than implied by "make the right side match".

use crate::compare::{Comparison, Verdict};

/// What a sync from one tree to another would do.
///
/// The direction is not in the method. The caller names a source and a
/// destination, and the method says how strictly the destination should be
/// made to match: `Update` brings across what it is missing, `Mirror` also
/// removes what it has that the source does not. Encoding "left" and "right"
/// in the method instead would make `--from a --to b` and `--from b --to a`
/// differ by a name the caller has to keep straight, and a mirror that copies
/// in the wrong direction is worse than no mirror.

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncMethod {
    /// Copy what the destination is missing, and what the source has newer.
    /// Removes nothing: the safe method, and the one to reach for first.
    Update,
    /// Copy both ways: the newer side wins. Not idempotent when a file
    /// differs and neither side is newer, because both copies happen.
    UpdateBoth,
    /// Make the destination exactly match the source, **including deleting
    /// what only the destination has**.
    Mirror,
}

impl SyncMethod {
    pub const ALL: [SyncMethod; 3] = [
        SyncMethod::Update,
        SyncMethod::UpdateBoth,
        SyncMethod::Mirror,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SyncMethod::Update => "update",
            SyncMethod::UpdateBoth => "update-both",
            SyncMethod::Mirror => "mirror",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|method| method.name() == value.trim().to_ascii_lowercase())
    }

    /// Whether this method removes anything. The answer belongs in the plan
    /// header, not only in its action list: a caller that wants to know
    /// whether to be frightened should not have to read the steps.
    pub fn deletes(self) -> bool {
        matches!(self, SyncMethod::Mirror)
    }
}

/// One thing the sync would do.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum SyncAction {
    /// Put the source's bytes at the destination.
    Copy {
        from: String,
        to: String,
        bytes: u64,
        reason: CopyReason,
    },
    /// Remove what is at the destination.
    Delete { path: String, bytes: u64 },
}

/// Why a copy is in the plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CopyReason {
    /// The destination does not have this path.
    Missing,
    /// The source is newer, under the comparison's tolerance.
    Newer,
    /// The two differ, and neither side is newer - the caller asked for a copy
    /// anyway because the method is bidirectional or because it forced the
    /// direction.
    Differs,
}

/// What a sync would do, before anyone decides to do it.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SyncPlan {
    pub method: SyncMethod,
    /// True when the plan contains deletions. Named separately so a caller
    /// rendering a confirmation sees it without counting steps.
    pub deletes: bool,
    pub actions: Vec<SyncAction>,
    pub copied_bytes: u64,
    pub deleted_bytes: u64,
    /// Paths the comparison could not decide, carried through untouched. A
    /// plan that silently omits them reads as a complete answer.
    pub unresolved: Vec<String>,
    /// Paths the caller kept out of the plan on purpose, with the reason.
    /// Named rather than dropped: a plan that quietly skips something reads
    /// as a complete answer, and the one thing a sync must never copy is the
    /// live index it is reading.
    pub excluded: Vec<PlanExclusion>,
}

/// A path the plan deliberately does not act on.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct PlanExclusion {
    pub path: String,
    pub reason: ExcludeReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExcludeReason {
    /// The index this comparison was read from. Copying it would duplicate a
    /// live database, and mirroring a tree that has one would delete the
    /// other tree's.
    IndexData,
}

impl SyncPlan {
    /// Whether the plan would change anything at all.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// The count a person asks for before agreeing to something.
    pub fn deletions(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| matches!(action, SyncAction::Delete { .. }))
            .count()
    }

    pub fn copies(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| matches!(action, SyncAction::Copy { .. }))
            .count()
    }
}

/// Builds the plan a method would produce from a comparison.
///
/// `from_root` is the tree being treated as the source and `to_root` the one
/// being corrected; every action names a real path under the root it touches,
/// so a plan reads as something a person can check against their file manager.
///
/// The contract callers rely on: `from_root` must be the tree the comparison
/// reports on the left, because `LeftOnly` and `RightOnly` are statements
/// about that comparison. Swapping the two roots swaps which side a plan
/// reads as the source, which is exactly what `--from a --to b` versus
/// `--from b --to a` is meant to do - but the caller has to make the swap
/// before the comparison, not after.
pub fn build_plan(
    method: SyncMethod,
    rows: &[Comparison<'_>],
    from_root: &str,
    to_root: &str,
    excluded: &[PlanExclusion],
) -> SyncPlan {
    let mut actions = Vec::new();
    let mut unresolved = Vec::new();
    let mut copied_bytes = 0_u64;
    let mut deleted_bytes = 0_u64;
    let is_excluded = |path: &str| {
        excluded
            .iter()
            .any(|entry| path == entry.path || path.starts_with(&format!("{}/", entry.path)))
    };

    for row in rows {
        // A path the caller excluded is named in the plan and acted on never,
        // which is a different thing from the path being absent.
        if is_excluded(&row.path) {
            continue;
        }
        let from_path = join(from_root, &row.path);
        let to_path = join(to_root, &row.path);
        match row.verdict {
            // `LeftOnly` means the source has it and the destination does
            // not, so under every method it travels across.
            Verdict::LeftOnly => {
                let bytes = left_bytes(row);
                actions.push(SyncAction::Copy {
                    from: from_path,
                    to: to_path,
                    bytes,
                    reason: CopyReason::Missing,
                });
                copied_bytes += bytes;
            }
            // `RightOnly` means only the destination has it. A mirror removes
            // it - that is what makes the destination match. An update leaves
            // it alone: removing what the caller did not ask to remove is the
            // part worth being careful about.
            Verdict::RightOnly => match method {
                SyncMethod::Mirror => {
                    let bytes = right_bytes(row);
                    actions.push(SyncAction::Delete {
                        path: to_path,
                        bytes,
                    });
                    deleted_bytes += bytes;
                }
                SyncMethod::Update | SyncMethod::UpdateBoth => {}
            },
            Verdict::Same { .. } => {}
            Verdict::Different { reason } => {
                use crate::compare::DifferentReason;
                if reason == DifferentReason::UnknownSize {
                    // One side could not report a size, so the plan cannot say
                    // what copying or deleting it would cost. Naming it beats
                    // leaving it out, which would read as a complete answer.
                    unresolved.push(row.path.clone());
                    continue;
                }
                match method {
                    // The source wins: the destination is corrected.
                    SyncMethod::Update | SyncMethod::Mirror => {
                        let bytes = right_bytes(row);
                        actions.push(SyncAction::Copy {
                            from: from_path.clone(),
                            to: to_path,
                            bytes,
                            reason: copy_reason_for(left_is_newer(row)),
                        });
                        copied_bytes += bytes;
                    }
                    // Whichever side is newer wins; when neither is, the
                    // destination wins, so the result is at least stable.
                    SyncMethod::UpdateBoth => {
                        let bytes = right_bytes(row);
                        actions.push(SyncAction::Copy {
                            from: from_path,
                            to: to_path,
                            bytes,
                            reason: copy_reason_for(left_is_newer(row)),
                        });
                        copied_bytes += bytes;
                    }
                }
            }
        }
    }
    actions.sort_by(|a, b| path_of(a).cmp(path_of(b)));
    SyncPlan {
        method,
        deletes: method.deletes(),
        actions,
        copied_bytes,
        deleted_bytes,
        unresolved,
        excluded: excluded.to_vec(),
    }
}

/// Newer wins. When neither side is newer the copy is still wanted - the
/// method decided that - and the plan says so rather than calling it an
/// update, because the source's bytes travelling over a file the destination
/// also has is not the same event as the destination being out of date.
fn copy_reason_for(newer: bool) -> CopyReason {
    if newer {
        CopyReason::Newer
    } else {
        CopyReason::Differs
    }
}

fn left_bytes(row: &Comparison<'_>) -> u64 {
    row.left.map_or(0, |node| node.subtree_bytes)
}

fn right_bytes(row: &Comparison<'_>) -> u64 {
    row.right.map_or(0, |node| node.subtree_bytes)
}

fn path_of(action: &SyncAction) -> &str {
    match action {
        SyncAction::Copy { to, .. } => to,
        SyncAction::Delete { path, .. } => path,
    }
}

fn left_is_newer(row: &Comparison<'_>) -> bool {
    match (row.left, row.right) {
        (Some(left), Some(right)) => {
            match (left.modified_unix_seconds, right.modified_unix_seconds) {
                (Some(left), Some(right)) => left > right,
                _ => false,
            }
        }
        _ => false,
    }
}

fn join(root: &str, relative: &str) -> String {
    if root.is_empty() {
        return relative.to_owned();
    }
    if root.ends_with('/') {
        format!("{root}{relative}")
    } else {
        format!("{root}/{relative}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::{DifferentReason, Evidence, Verdict};
    use crate::model::{DiskNode, NodeKind, ResourceLocator};

    fn file(id: u64, path: &str, bytes: u64, mtime: Option<i64>) -> DiskNode {
        DiskNode {
            id,
            parent_id: Some(1),
            locator: ResourceLocator::NativePath(path.to_owned()),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            kind: NodeKind::File,
            subtree_bytes: bytes,
            direct_bytes: bytes,
            size_known: true,
            files: 1,
            directories: 0,
            modified_unix_seconds: mtime,
            file_identity: None,
            category_hint: None,
            reclaim_hint: None,
            read_error: false,
        }
    }

    /// A comparison where the first argument's root is the source side and
    /// the second is the destination.
    struct Pair {
        rows: Vec<Comparison<'static>>,
    }

    /// `source_newer` decides which side of the differing file is newer, so a
    /// test can check that the source's bytes are the ones that travel.
    fn pair(source_newer: bool) -> Pair {
        let leak = |node: DiskNode| -> &'static DiskNode { Box::leak(Box::new(node)) };
        let source_only = leak(file(1, "/src/only-in-source.bin", 100, Some(500)));
        let dest_only = leak(file(2, "/dest/only-in-dest.bin", 200, Some(500)));
        let (older, newer) = if source_newer {
            (
                leak(file(3, "/dest/differs.bin", 300, Some(400))),
                leak(file(3, "/src/differs.bin", 300, Some(600))),
            )
        } else {
            (
                leak(file(3, "/src/differs.bin", 300, Some(400))),
                leak(file(3, "/dest/differs.bin", 300, Some(600))),
            )
        };
        let rows = vec![
            Comparison {
                path: "only-in-source.bin".into(),
                verdict: Verdict::LeftOnly,
                left: Some(source_only),
                right: None,
            },
            Comparison {
                path: "only-in-dest.bin".into(),
                verdict: Verdict::RightOnly,
                left: None,
                right: Some(dest_only),
            },
            Comparison {
                path: "differs.bin".into(),
                verdict: Verdict::Different {
                    reason: DifferentReason::Size,
                },
                left: Some(older),
                right: Some(newer),
            },
        ];
        Pair { rows }
    }

    fn plan_for(method: SyncMethod, source_newer: bool) -> SyncPlan {
        let pair = pair(source_newer);
        build_plan(method, &pair.rows, "/src", "/dest", &[])
    }

    fn deletions(plan: &SyncPlan) -> Vec<&SyncAction> {
        plan.actions
            .iter()
            .filter(|action| matches!(action, SyncAction::Delete { .. }))
            .collect()
    }

    fn copies(plan: &SyncPlan) -> Vec<&SyncAction> {
        plan.actions
            .iter()
            .filter(|action| matches!(action, SyncAction::Copy { .. }))
            .collect()
    }

    fn plan_deleted_paths(plan: &SyncPlan) -> Vec<String> {
        deletions(plan)
            .iter()
            .filter_map(|action| match action {
                SyncAction::Delete { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn update_copies_into_the_destination_and_deletes_nothing() {
        let plan = plan_for(SyncMethod::Update, true);
        assert!(!plan.deletes);
        assert!(deletions(&plan).is_empty());
        // Only the source knows are copied into the destination.
        let moved: Vec<&str> = copies(&plan)
            .iter()
            .filter_map(|action| match action {
                SyncAction::Copy { from, .. } => Some(from.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            moved.iter().all(|from| from.starts_with("/src")),
            "update never reads from the destination: {moved:?}"
        );
        assert_eq!(plan.copied_bytes, 100 + 300);
    }

    #[test]
    fn update_both_still_deletes_nothing() {
        let plan = plan_for(SyncMethod::UpdateBoth, true);
        assert!(!plan.deletes);
        assert!(deletions(&plan).is_empty());
    }

    #[test]
    fn mirroring_a_into_b_removes_what_is_only_in_b() {
        // `--from a --to b --method mirror` is "make b look like a": the
        // file only in b is the one that goes.
        let plan = plan_for(SyncMethod::Mirror, true);
        assert!(plan.deletes, "the header says so before the steps are read");
        let gone = deletions(&plan);
        assert_eq!(gone.len(), 1, "exactly one file is only in the destination");
        assert!(
            matches!(gone[0], SyncAction::Delete { path, bytes: 200 } if path == "/dest/only-in-dest.bin"),
            "{gone:?}"
        );
        assert_eq!(plan.deleted_bytes, 200);
    }

    #[test]
    fn mirroring_a_into_b_keeps_what_is_only_in_a_by_copying_it_over() {
        let plan = plan_for(SyncMethod::Mirror, true);
        assert!(
            copies(&plan).iter().any(|action| matches!(
                action,
                SyncAction::Copy { from, to, .. }
                    if from == "/src/only-in-source.bin" && to == "/dest/only-in-source.bin"
            )),
            "the source-only file travels across rather than being deleted: {:?}",
            plan.actions
        );
    }

    /// The same tree pair seen the other way round: the source's files are
    /// now on the comparison's right, which is what a caller asking to
    /// mirror b into a necessarily has.
    fn pair_from_the_other_side(source_newer: bool) -> Pair {
        let forward = pair(source_newer);
        let rows = forward
            .rows
            .into_iter()
            .map(|row| {
                // Swapping the sides swaps what LeftOnly and RightOnly mean:
                // a path only the left had is now a path only the right has.
                let verdict = match row.verdict {
                    Verdict::LeftOnly => Verdict::RightOnly,
                    Verdict::RightOnly => Verdict::LeftOnly,
                    other => other,
                };
                Comparison {
                    path: row.path,
                    verdict,
                    left: row.right,
                    right: row.left,
                }
            })
            .collect();
        Pair { rows }
    }

    #[test]
    fn the_direction_is_the_callers_not_the_methods() {
        // `--from a --to b` and `--from b --to a` are the same comparison
        // with the two sides swapped - both the roots and the rows - and they
        // must remove opposite files. The caller owns that swap: a plan
        // builder told `from_root` is the comparison's left, and handing it a
        // right instead would make it copy from a tree the file is not in.
        let forwards = build_plan(SyncMethod::Mirror, &pair(true).rows, "/src", "/dest", &[]);
        let backwards = build_plan(
            SyncMethod::Mirror,
            &pair_from_the_other_side(true).rows,
            "/dest",
            "/src",
            &[],
        );
        assert_eq!(
            plan_deleted_paths(&forwards),
            vec!["/dest/only-in-dest.bin"]
        );
        assert_eq!(
            plan_deleted_paths(&backwards),
            vec!["/src/only-in-source.bin"]
        );
    }

    #[test]
    fn the_source_wins_for_a_differing_file() {
        let plan = plan_for(SyncMethod::Update, true);
        let differing = copies(&plan)
            .iter()
            .find_map(|action| match action {
                SyncAction::Copy { to, .. } if to.ends_with("differs.bin") => Some(*action),
                _ => None,
            })
            .expect("the differing file is copied");
        assert!(
            matches!(differing, SyncAction::Copy { from, .. } if from.starts_with("/src")),
            "the source is the side a sync corrects towards: {differing:?}"
        );
    }

    #[test]
    fn a_row_nobody_could_measure_is_named_rather_than_acted_on() {
        let node: &'static DiskNode = Box::leak(Box::new(file(9, "/src/vague.bin", 0, None)));
        let rows = vec![Comparison {
            path: "vague.bin".into(),
            verdict: Verdict::Different {
                reason: DifferentReason::UnknownSize,
            },
            left: Some(node),
            right: Some(node),
        }];
        let plan = build_plan(SyncMethod::Mirror, &rows, "/src", "/dest", &[]);
        assert!(plan.actions.is_empty(), "an unknown size is not an action");
        assert_eq!(plan.unresolved, vec!["vague.bin".to_owned()]);
    }

    #[test]
    fn identical_rows_produce_an_empty_plan() {
        let node: &'static DiskNode = Box::leak(Box::new(file(1, "/src/x", 5, Some(1))));
        let rows = vec![Comparison {
            path: "x".into(),
            verdict: Verdict::Same {
                evidence: Evidence::Metadata,
            },
            left: Some(node),
            right: Some(node),
        }];
        assert!(build_plan(SyncMethod::Mirror, &rows, "/src", "/dest", &[]).is_empty());
    }

    #[test]
    fn every_method_round_trips_through_its_name() {
        for method in SyncMethod::ALL {
            assert_eq!(SyncMethod::parse(method.name()), Some(method));
        }
        assert_eq!(SyncMethod::parse("MIRROR"), Some(SyncMethod::Mirror));
        assert_eq!(SyncMethod::parse("sideways"), None);
        assert!(SyncMethod::Mirror.deletes());
        assert!(!SyncMethod::Update.deletes());
        assert!(!SyncMethod::UpdateBoth.deletes());
    }

    #[test]
    fn an_excluded_path_is_named_and_never_acted_on() {
        let leak = |node: DiskNode| -> &'static DiskNode { Box::leak(Box::new(node)) };
        let index = leak(file(7, "/src/.diskgraph/diskgraph.sqlite", 5_000, Some(1)));
        let ordinary = leak(file(8, "/src/notes.md", 100, Some(1)));
        let rows = vec![
            Comparison {
                path: ".diskgraph/diskgraph.sqlite".into(),
                verdict: Verdict::LeftOnly,
                left: Some(index),
                right: None,
            },
            Comparison {
                path: ".diskgraph".into(),
                verdict: Verdict::LeftOnly,
                left: Some(index),
                right: None,
            },
            Comparison {
                path: "notes.md".into(),
                verdict: Verdict::LeftOnly,
                left: Some(ordinary),
                right: None,
            },
        ];
        let exclusion = [PlanExclusion {
            path: ".diskgraph".into(),
            reason: ExcludeReason::IndexData,
        }];
        let plan = build_plan(SyncMethod::Update, &rows, "/src", "/dest", &exclusion);
        // Neither the directory itself nor anything under it is copied: a
        // plan that duplicated a live database, or mirrored one tree's index
        // over another's, would be the worst thing this command could do.
        assert!(
            plan.actions
                .iter()
                .all(|action| !path_of(action).contains(".diskgraph")),
            "{:?}",
            plan.actions
        );
        assert_eq!(plan.copies(), 1, "the one file that is not the index");
        assert_eq!(plan.excluded, exclusion, "and the exclusion is reported");
    }

    #[test]
    fn excluding_one_side_leaves_the_others_untouched() {
        let leak = |node: DiskNode| -> &'static DiskNode { Box::leak(Box::new(node)) };
        let rows = vec![Comparison {
            path: "notes.md".into(),
            verdict: Verdict::RightOnly,
            left: None,
            right: Some(leak(file(1, "/dest/notes.md", 10, Some(1)))),
        }];
        let plan = build_plan(
            SyncMethod::Mirror,
            &rows,
            "/src",
            "/dest",
            &[PlanExclusion {
                path: ".diskgraph".into(),
                reason: ExcludeReason::IndexData,
            }],
        );
        assert_eq!(
            plan.deletions(),
            1,
            "an unrelated exclusion changes nothing"
        );
    }

    #[test]
    fn paths_are_joined_without_doubling_separators() {
        let node: &'static DiskNode = Box::leak(Box::new(file(1, "/src/x", 5, Some(1))));
        let rows = vec![Comparison {
            path: "x".into(),
            verdict: Verdict::LeftOnly,
            left: Some(node),
            right: None,
        }];
        let plan = build_plan(SyncMethod::Update, &rows, "/src/", "/dest/", &[]);
        assert!(plan.actions.iter().all(|action| match action {
            SyncAction::Copy { from, to, .. } => !from.contains("//") && !to.contains("//"),
            SyncAction::Delete { path, .. } => !path.contains("//"),
        }));
    }
}
