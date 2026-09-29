//! Duplicate suspects (CT-03, P7 task 8.3). Grouping is a metadata-only
//! review queue: equal size is a *suspect*, never a verdict, and only a
//! separately authorized content confirmation may upgrade a group — the
//! result of which can never authorize a deletion by itself.

/// How two same-size objects relate, as far as metadata alone can say.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentRelation {
    /// Same file identity (device:inode on unix): one file, many names.
    /// Cleaning one name does not free the other's bytes.
    HardLinked,
    /// Same size, distinct or unknown identities: a suspect only.
    SameSizeSuspect,
}

/// One metadata-suspect group: at least two objects of equal size, described
/// without claiming anything about their content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuspectGroup {
    /// The size every member shares.
    pub size_bytes: u64,
    /// Members as (node id, optional file identity). Two members with the
    /// same Some(identity) are hard links to one file, reported as such.
    pub members: Vec<(u64, ContentRelation)>,
}

/// A confirmed duplicate set: produced only by a content-confirmation job
/// that ran under its own authorization and budget, never by this grouping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmedSet {
    /// The digest every confirmed member shares.
    pub digest: String,
    /// Member node ids whose content confirmed equal and stable.
    pub members: Vec<u64>,
}

/// Groups same-size objects into review-only suspects. Sizes are the anchor:
/// objects of different sizes can never be content-duplicates. Members whose
/// file identity matches another member are annotated as hard links, and a
/// group made up entirely of one identity is not a duplication at all.
pub fn suspect_groups(objects: &[(u64, u64, Option<&str>)]) -> Vec<SuspectGroup>
where
{
    let mut by_size: std::collections::BTreeMap<u64, Vec<(u64, Option<&str>)>> =
        std::collections::BTreeMap::new();
    for &(node_id, size, identity) in objects {
        by_size.entry(size).or_default().push((node_id, identity));
    }
    let mut groups = Vec::new();
    for (size, members) in by_size {
        if size == 0 || members.len() < 2 {
            // An empty file matches every other empty file; that is not a
            // duplication worth a review.
            continue;
        }
        // Collapse members that share one identity into hard-link groups.
        let mut by_identity: std::collections::BTreeMap<Option<&str>, Vec<u64>> =
            std::collections::BTreeMap::new();
        for (node_id, identity) in members {
            by_identity.entry(identity).or_default().push(node_id);
        }
        let mut annotated: Vec<(u64, ContentRelation)> = Vec::new();
        // Distinct files inside the group: each known identity is one file;
        // an unknown identity can only stand for its own node.
        let mut distinct_files: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        // An identity shared by two names is a hard link; an identity only
        // one name holds is just a file, still a content suspect.
        let shared: std::collections::HashSet<&str> = by_identity
            .iter()
            .filter(|(_, nodes)| nodes.len() > 1)
            .filter_map(|(identity, _)| *identity)
            .collect();
        for (identity, nodes) in by_identity {
            let relation = match identity {
                Some(ref id) if shared.contains(id) => {
                    distinct_files.insert((*id).to_owned());
                    ContentRelation::HardLinked
                }
                _ => {
                    match &identity {
                        Some(id) => {
                            distinct_files.insert((*id).to_owned());
                        }
                        None => {
                            for node_id in &nodes {
                                distinct_files.insert(format!("node:{node_id}"));
                            }
                        }
                    }
                    ContentRelation::SameSizeSuspect
                }
            };
            for node_id in nodes {
                annotated.push((node_id, relation));
            }
        }
        annotated.sort_by_key(|(node_id, _)| *node_id);
        // Every member sharing one identity means one file with several
        // names: not duplicates, just links.
        if distinct_files.len() < 2 {
            continue;
        }
        groups.push(SuspectGroup {
            size_bytes: size,
            members: annotated,
        });
    }
    groups
}

/// Upgrades a suspect group with a content confirmation. This is the only
/// path from "suspect" to "confirmed", and it refuses to confirm anything
/// whose confirmation was not stable (CT-03).
///
/// `confirmations` maps node id to the digest its confirmation produced; a
/// missing or unstable member simply does not join the confirmed set.
pub fn confirm_group(
    group: &SuspectGroup,
    confirmations: &[(u64, Option<String>)],
) -> Option<ConfirmedSet> {
    let member_ids: std::collections::HashSet<u64> =
        group.members.iter().map(|(node_id, _)| *node_id).collect();
    let mut by_digest: std::collections::BTreeMap<&str, Vec<u64>> =
        std::collections::BTreeMap::new();
    for &(node_id, ref digest) in confirmations {
        if let Some(digest) = digest
            && member_ids.contains(&node_id)
        {
            by_digest.entry(digest).or_default().push(node_id);
        }
    }
    let (digest, members) = by_digest
        .into_iter()
        .max_by_key(|(_, members)| members.len())?;
    if members.len() < 2 {
        return None;
    }
    Some(ConfirmedSet {
        digest: digest.to_owned(),
        members,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_size_is_a_suspect_never_a_verdict() {
        // Two files, same size, unknown content: the only honest claim is
        // "suspect" (CT-03).
        let groups = suspect_groups(&[(1, 4096, None), (2, 4096, None), (3, 8192, None)]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].size_bytes, 4096);
        assert_eq!(
            groups[0].members,
            vec![
                (1, ContentRelation::SameSizeSuspect),
                (2, ContentRelation::SameSizeSuspect)
            ]
        );
    }

    #[test]
    fn hard_links_are_one_file_reported_as_links() {
        let groups = suspect_groups(&[
            (1, 4096, Some("dev:11")),
            (2, 4096, Some("dev:11")),
            (3, 4096, Some("dev:22")),
        ]);
        assert_eq!(groups.len(), 1);
        assert!(
            groups[0]
                .members
                .contains(&(1, ContentRelation::HardLinked))
        );
        assert!(
            groups[0]
                .members
                .contains(&(3, ContentRelation::SameSizeSuspect))
        );
        // All members sharing one identity: one file with several names.
        let linked = suspect_groups(&[(1, 4096, Some("d:i")), (2, 4096, Some("d:i"))]);
        assert!(linked.is_empty(), "hard links alone are not duplicates");
    }

    #[test]
    fn different_sizes_and_empty_files_never_group() {
        assert!(suspect_groups(&[(1, 1, None), (2, 2, None)]).is_empty());
        assert!(suspect_groups(&[(1, 0, None), (2, 0, None)]).is_empty());
    }

    #[test]
    fn confirmation_requires_two_stable_equal_digests() {
        let group = SuspectGroup {
            size_bytes: 10,
            members: vec![
                (1, ContentRelation::SameSizeSuspect),
                (2, ContentRelation::SameSizeSuspect),
            ],
        };
        // One confirmed member is not a duplicate set.
        assert!(confirm_group(&group, &[(1, Some("d1".into())), (2, None)]).is_none());
        // Two members, same digest: confirmed — but this is a review record,
        // never a deletion authorization.
        let confirmed =
            confirm_group(&group, &[(1, Some("d1".into())), (2, Some("d1".into()))]).unwrap();
        assert_eq!(confirmed.members, vec![1, 2]);
        assert_eq!(confirmed.digest, "d1");
    }
}
