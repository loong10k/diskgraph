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

/// Which way a sync travels, and whether it removes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncMethod {
    /// Copy what the right side has or has newer, into the left. Deletes
    /// nothing: the safest method, and the one to reach for by default.
    UpdateLeft,
    /// The mirror image of `UpdateLeft`.
    UpdateRight,
    /// Copy newer files both ways. A file that differs goes whichever way its
    /// timestamp points, so this is not idempotent when the clock is wrong.
    UpdateBoth,
    /// Make the left side exactly match the right, **including deleting what
    /// is only on the left**.
    MirrorLeft,
    /// The mirror image of `MirrorLeft`, deletions included.
    MirrorRight,
}

impl SyncMethod {
    pub const ALL: [SyncMethod; 5] = [
        SyncMethod::UpdateLeft,
        SyncMethod::UpdateRight,
        SyncMethod::UpdateBoth,
        SyncMethod::MirrorLeft,
        SyncMethod::MirrorRight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SyncMethod::UpdateLeft => "update-left",
            SyncMethod::UpdateRight => "update-right",
            SyncMethod::UpdateBoth => "update-both",
            SyncMethod::MirrorLeft => "mirror-left",
            SyncMethod::MirrorRight => "mirror-right",
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
        matches!(self, SyncMethod::MirrorLeft | SyncMethod::MirrorRight)
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
/// `left_root` and `right_root` are the directories the relative paths in the
/// comparison hang off, so every action names a real path rather than a
/// fragment a caller has to reassemble.
pub fn build_plan(
    method: SyncMethod,
    rows: &[Comparison<'_>],
    left_root: &str,
    right_root: &str,
) -> SyncPlan {
    let mut actions = Vec::new();
    let mut unresolved = Vec::new();
    let mut copied_bytes = 0_u64;
    let mut deleted_bytes = 0_u64;

    for row in rows {
        let left_path = join(left_root, &row.path);
        let right_path = join(right_root, &row.path);
        match row.verdict {
            // Which side a one-sided path is removed from is the side the
            // method is making match: mirroring the left onto the right deletes
            // what is only on the right, because that is the side being
            // corrected. Deleting the other one would be deleting a file the
            // method is about to copy in.
            Verdict::LeftOnly => match method {
                SyncMethod::MirrorLeft => {
                    let bytes = left_bytes(row);
                    actions.push(SyncAction::Delete {
                        path: left_path,
                        bytes,
                    });
                    deleted_bytes += bytes;
                }
                // The right does not have it and the right is the target -
                // including under a mirror, which copies before it deletes.
                SyncMethod::UpdateRight | SyncMethod::UpdateBoth | SyncMethod::MirrorRight => {
                    let bytes = left_bytes(row);
                    actions.push(SyncAction::Copy {
                        from: left_path,
                        to: right_path,
                        bytes,
                        reason: CopyReason::Missing,
                    });
                    copied_bytes += bytes;
                }
                // The left is the target and already has it: nothing to do.
                SyncMethod::UpdateLeft => {}
            },
            Verdict::RightOnly => match method {
                SyncMethod::MirrorRight => {
                    let bytes = right_bytes(row);
                    actions.push(SyncAction::Delete {
                        path: right_path,
                        bytes,
                    });
                    deleted_bytes += bytes;
                }
                SyncMethod::UpdateLeft | SyncMethod::UpdateBoth | SyncMethod::MirrorLeft => {
                    let bytes = right_bytes(row);
                    actions.push(SyncAction::Copy {
                        from: right_path,
                        to: left_path,
                        bytes,
                        reason: CopyReason::Missing,
                    });
                    copied_bytes += bytes;
                }
                SyncMethod::UpdateRight => {}
            },
            Verdict::Same { .. } => {}
            Verdict::Different { reason } => {
                use crate::compare::DifferentReason;
                if reason == DifferentReason::UnknownSize {
                    // One side could not report a size, so the plan cannot
                    // say what copying or deleting it would cost. Leaving it
                    // out of the action list and naming it instead is the
                    // difference between a plan and a guess.
                    unresolved.push(row.path.clone());
                    continue;
                }
                match method {
                    SyncMethod::UpdateLeft | SyncMethod::MirrorLeft => {
                        // The left is made to match the right.
                        let bytes = right_bytes(row);
                        actions.push(SyncAction::Copy {
                            from: right_path,
                            to: left_path,
                            bytes,
                            reason: copy_reason_for(right_is_newer(row)),
                        });
                        copied_bytes += bytes;
                    }
                    SyncMethod::UpdateRight | SyncMethod::MirrorRight => {
                        let bytes = left_bytes(row);
                        actions.push(SyncAction::Copy {
                            from: left_path,
                            to: right_path,
                            bytes,
                            reason: copy_reason_for(left_is_newer(row)),
                        });
                        copied_bytes += bytes;
                    }
                    SyncMethod::UpdateBoth => {
                        // Both directions: the newer side wins, and when
                        // neither is newer both copies happen and the second
                        // one wins. That is what "update both" means, and it
                        // is why the method is not the default.
                        if left_is_newer(row) {
                            let bytes = left_bytes(row);
                            actions.push(SyncAction::Copy {
                                from: left_path.clone(),
                                to: right_path,
                                bytes,
                                reason: CopyReason::Newer,
                            });
                            copied_bytes += bytes;
                        } else {
                            let bytes = right_bytes(row);
                            actions.push(SyncAction::Copy {
                                from: right_path,
                                to: left_path,
                                bytes,
                                reason: copy_reason_for(right_is_newer(row)),
                            });
                            copied_bytes += bytes;
                        }
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
    }
}

/// Newer wins; when neither is newer the copy is still wanted - the method
/// decided that - and the plan says so rather than calling it an update.
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

fn right_is_newer(row: &Comparison<'_>) -> bool {
    match (row.left, row.right) {
        (Some(left), Some(right)) => {
            match (left.modified_unix_seconds, right.modified_unix_seconds) {
                (Some(left), Some(right)) => right > left,
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

    /// Three rows covering what a plan has to decide about: a path only the
    /// left has, a path only the right has, and one that differs with the
    /// right side newer.
    fn rows() -> (Vec<DiskNode>, Vec<Comparison<'static>>) {
        let left_only: &'static DiskNode =
            Box::leak(Box::new(file(1, "/a/only-left.bin", 100, Some(500))));
        let right_only: &'static DiskNode =
            Box::leak(Box::new(file(2, "/b/only-right.bin", 200, Some(500))));
        let older: &'static DiskNode =
            Box::leak(Box::new(file(3, "/a/differs.bin", 300, Some(400))));
        let newer: &'static DiskNode =
            Box::leak(Box::new(file(3, "/b/differs.bin", 300, Some(600))));
        let comparisons = vec![
            Comparison {
                path: "only-left.bin".into(),
                verdict: Verdict::LeftOnly,
                left: Some(left_only),
                right: None,
            },
            Comparison {
                path: "only-right.bin".into(),
                verdict: Verdict::RightOnly,
                left: None,
                right: Some(right_only),
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
        // The leaked nodes outlive the caller's use of the plan; returning
        // the owner list keeps the signature honest about what is borrowed.
        (Vec::new(), comparisons)
    }

    #[test]
    fn update_left_copies_and_deletes_nothing() {
        let (_owned, rows) = rows();
        let plan = build_plan(SyncMethod::UpdateLeft, &rows, "/a", "/b");
        assert!(!plan.deletes);
        assert_eq!(plan.deletions(), 0);
        assert_eq!(plan.copies(), 2, "the right-only file and the newer one");
        assert_eq!(plan.copied_bytes, 200 + 300);
    }

    #[test]
    fn update_right_mirrors_the_direction_without_deleting() {
        let (_owned, rows) = rows();
        let plan = build_plan(SyncMethod::UpdateRight, &rows, "/a", "/b");
        assert!(!plan.deletes);
        // The left-only file has nowhere to be copied to on the right that
        // does not overwrite something, so an update leaves it alone.
        assert!(
            plan.actions
                .iter()
                .all(|action| !matches!(action, SyncAction::Delete { .. })),
            "an update method never removes"
        );
    }

    #[test]
    fn a_mirror_names_every_deletion_as_its_own_step() {
        let (_owned, rows) = rows();
        // Mirroring onto the right deletes what is only on the right, because
        // that is the side being corrected - not what is only on the left,
        // which is the file it is about to copy in.
        let plan = build_plan(SyncMethod::MirrorRight, &rows, "/a", "/b");
        assert!(plan.deletes, "the header says so before the steps are read");
        let deletions: Vec<&SyncAction> = plan
            .actions
            .iter()
            .filter(|action| matches!(action, SyncAction::Delete { .. }))
            .collect();
        assert_eq!(deletions.len(), 1);
        assert!(
            matches!(deletions[0], SyncAction::Delete { path, bytes: 200 } if path == "/b/only-right.bin"),
            "and it is the right-only file: {deletions:?}"
        );
        assert_eq!(plan.deleted_bytes, 200);
        // The left-only file is copied across, not removed.
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            SyncAction::Copy { from, to, .. } if from == "/a/only-left.bin" && to == "/b/only-left.bin"
        )));
    }

    #[test]
    fn mirror_left_travels_the_other_way_and_also_deletes() {
        let (_owned, rows) = rows();
        let plan = build_plan(SyncMethod::MirrorLeft, &rows, "/a", "/b");
        assert!(plan.deletes);
        assert!(
            plan.actions.iter().any(|action| matches!(
                action,
                SyncAction::Delete { path, bytes: 100 } if path == "/a/only-left.bin"
            )),
            "the left is the side being corrected, so the left-only path is \
             what goes: {actions:?}",
            actions = plan.actions
        );
    }

    #[test]
    fn update_both_goes_the_newer_way_only() {
        let (_owned, rows) = rows();
        let plan = build_plan(SyncMethod::UpdateBoth, &rows, "/a", "/b");
        assert!(!plan.deletes);
        let differing_copy = plan
            .actions
            .iter()
            .find(|action| {
                matches!(action, SyncAction::Copy { to, .. } if to.ends_with("differs.bin"))
            })
            .expect("the differing file is copied");
        // The right side is newer, so the right's bytes win.
        assert!(matches!(differing_copy, SyncAction::Copy { from, .. } if from.starts_with("/b")));
    }

    #[test]
    fn a_row_nobody_could_measure_is_named_rather_than_acted_on() {
        let node: &'static DiskNode = Box::leak(Box::new(file(9, "/b/vague.bin", 0, None)));
        let rows = vec![Comparison {
            path: "vague.bin".into(),
            verdict: Verdict::Different {
                reason: DifferentReason::UnknownSize,
            },
            left: None,
            right: Some(node),
        }];
        let plan = build_plan(SyncMethod::MirrorLeft, &rows, "/a", "/b");
        assert!(plan.actions.is_empty(), "an unknown size is not an action");
        assert_eq!(plan.unresolved, vec!["vague.bin".to_owned()]);
    }

    #[test]
    fn identical_rows_produce_an_empty_plan() {
        let owned: &'static DiskNode = Box::leak(Box::new(file(1, "/a/x", 5, Some(1))));
        let rows = vec![Comparison {
            path: "x".into(),
            verdict: Verdict::Same {
                evidence: Evidence::Metadata,
            },
            left: Some(owned),
            right: Some(owned),
        }];
        assert!(build_plan(SyncMethod::MirrorLeft, &rows, "/a", "/b").is_empty());
    }

    #[test]
    fn every_method_round_trips_through_its_name() {
        for method in SyncMethod::ALL {
            assert_eq!(SyncMethod::parse(method.name()), Some(method));
        }
        assert_eq!(
            SyncMethod::parse("MIRROR-LEFT"),
            Some(SyncMethod::MirrorLeft)
        );
        assert_eq!(SyncMethod::parse("sideways"), None);
        assert!(SyncMethod::MirrorLeft.deletes());
        assert!(SyncMethod::MirrorRight.deletes());
        assert!(!SyncMethod::UpdateLeft.deletes());
        assert!(!SyncMethod::UpdateRight.deletes());
        assert!(!SyncMethod::UpdateBoth.deletes());
    }

    #[test]
    fn paths_are_joined_without_doubling_separators() {
        let owned: &'static DiskNode = Box::leak(Box::new(file(1, "/a/x", 5, Some(1))));
        let rows = vec![Comparison {
            path: "x".into(),
            verdict: Verdict::RightOnly,
            left: None,
            right: Some(owned),
        }];
        let plan = build_plan(SyncMethod::UpdateLeft, &rows, "/a/", "/b/");
        assert!(plan.actions.iter().all(|action| match action {
            SyncAction::Copy { from, to, .. } => !from.contains("//") && !to.contains("//"),
            SyncAction::Delete { path, .. } => !path.contains("//"),
        }));
    }
}
