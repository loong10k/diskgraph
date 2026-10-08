use crate::tests::graph;
use crate::{SqliteSnapshotStore, StoreError};
use diskgraph_core::{
    JobRequestAuthority, LocatorEncoding, LocatorKind, Permission, PrincipalId, QualifiedLocator,
    ResourceLocator,
};
use rusqlite::hooks::Action;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn fenced_publication_excludes_post_commit_checkpoint_but_compatibility_retains_it() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let checkpoint = Arc::new(AtomicBool::new(false));
    let seen = checkpoint.clone();
    store
        .connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if let AuthAction::Pragma {
                pragma_name: "wal_checkpoint",
                ..
            } = context.action
            {
                seen.store(true, Ordering::SeqCst);
            }
            Authorization::Allow
        }))
        .unwrap();
    let first = graph("fenced-maintenance", 100);
    store
        .append_staging_nodes("fenced-maintenance:1", &first.nodes)
        .unwrap();
    store
        .publish_revision_owned_with_batch_checked(
            "fenced-maintenance:1",
            &first,
            ("fenced-revision", 1),
            Some(("server", "scope")),
            None,
            || Ok(()),
        )
        .unwrap();
    assert!(
        !checkpoint.load(Ordering::SeqCst),
        "fenced commit must return before explicit post-commit maintenance"
    );
    assert!(store.revision("fenced-revision").is_ok());
    let second = graph("compatibility-maintenance", 100);
    store
        .publish_revision("compatibility-job", &second, "compatibility-revision", 2)
        .unwrap();
    assert!(
        checkpoint.load(Ordering::SeqCst),
        "positive control: trusted compatibility publication must retain checkpoint"
    );
}

fn locators(graph: &diskgraph_core::DiskGraph) -> Vec<QualifiedLocator> {
    graph
        .nodes
        .iter()
        .map(|node| {
            let ResourceLocator::NativePath(path) = &node.locator else {
                unreachable!()
            };
            QualifiedLocator::from_parts(
                LocatorKind::NativePath,
                LocatorEncoding::UnixBytes,
                path.as_bytes().to_vec(),
                path.clone(),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn publication_denial_after_real_seal_rolls_back_latest_ownership_and_staging_cleanup() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let previous = graph("previous", 100);
    store
        .publish_revision("old-job", &previous, "previous-revision", 1)
        .unwrap();
    let next = graph("next", 200);
    store
        .append_staging_nodes("next-job:1", &next.nodes)
        .unwrap();
    let sealed = Arc::new(AtomicBool::new(false));
    let latest_written = Arc::new(AtomicBool::new(false));
    let hook_sealed = sealed.clone();
    let hook_latest = latest_written.clone();
    store
        .connection
        .update_hook(Some(move |action, _: &str, table: &str, _: i64| {
            if table == "graph_revisions" && action == Action::SQLITE_UPDATE {
                hook_sealed.store(true, Ordering::SeqCst);
            }
            if table == "latest_revision" {
                hook_latest.store(true, Ordering::SeqCst);
            }
        }))
        .unwrap();
    let result = store.publish_revision_owned_with_batch_checked(
        "next-job:1",
        &next,
        ("next-revision", 2),
        Some(("server", "scope")),
        None,
        || {
            if sealed.load(Ordering::SeqCst) {
                Err(StoreError::Conflict("terminal authority denied".into()))
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(StoreError::Conflict(_))));
    assert!(
        sealed.load(Ordering::SeqCst),
        "real final seal SQL must execute before rejection"
    );
    assert!(latest_written.load(Ordering::SeqCst));
    assert_eq!(
        store
            .latest_revision_for_root(&previous.snapshot.root)
            .unwrap()
            .as_deref(),
        Some("previous-revision")
    );
    assert!(matches!(
        store.revision("next-revision"),
        Err(StoreError::RevisionNotFound(_))
    ));
    assert_eq!(store.revision_ownership("next-revision").unwrap(), None);
    assert_eq!(store.staging_node_count("next-job:1").unwrap(), 2);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM snapshots WHERE id='next'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn real_token_expiry_during_final_sql_cannot_commit_a_revision() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let next = graph("expired", 200);
    store
        .append_staging_nodes("expired-job:1", &next.nodes)
        .unwrap();
    let now = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    let expiry = now() + 2;
    let authority = JobRequestAuthority::authenticated_remote(
        PrincipalId::new("alice").unwrap(),
        "issuer",
        "http",
        vec![Permission::IndexWrite],
        expiry,
    )
    .unwrap();
    let sealed = Arc::new(AtomicBool::new(false));
    let hook_sealed = sealed.clone();
    store
        .connection
        .update_hook(Some(move |action, _: &str, table: &str, _: i64| {
            if table == "graph_revisions" && action == Action::SQLITE_UPDATE {
                hook_sealed.store(true, Ordering::SeqCst);
                let bound = Instant::now() + Duration::from_secs(4);
                while SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    < expiry
                {
                    assert!(Instant::now() < bound);
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }))
        .unwrap();
    let result = store.publish_revision_owned_with_batch_checked(
        "expired-job:1",
        &next,
        ("expired-revision", 2),
        Some(("server", "scope")),
        None,
        || {
            authority
                .validate_at(now())
                .map_err(|_| StoreError::Conflict("original token expired".into()))
        },
    );
    assert!(sealed.load(Ordering::SeqCst));
    assert!(matches!(result, Err(StoreError::Conflict(_))));
    assert_eq!(
        store.latest_revision_for_root(&next.snapshot.root).unwrap(),
        None
    );
    assert!(matches!(
        store.revision("expired-revision"),
        Err(StoreError::RevisionNotFound(_))
    ));
    assert_eq!(store.staging_node_count("expired-job:1").unwrap(), 2);
}

#[test]
fn staging_final_denial_rolls_back_rows_actually_written_before_commit() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let input = graph("staging", 100);
    let locations = locators(&input);
    let written = Arc::new(AtomicBool::new(false));
    let hook_written = written.clone();
    store
        .connection
        .update_hook(Some(move |action, _: &str, table: &str, _: i64| {
            if table == "scan_staging_search" && action == Action::SQLITE_INSERT {
                hook_written.store(true, Ordering::SeqCst);
            }
        }))
        .unwrap();
    let result = store.append_staging_observed_iter_checked(
        "staging-job:1",
        std::iter::once((&input.nodes[0], &locations[0], None, None, None)),
        || {
            if written.load(Ordering::SeqCst) {
                Err(StoreError::Conflict("cancelled before commit".into()))
            } else {
                Ok(())
            }
        },
    );
    assert!(
        written.load(Ordering::SeqCst),
        "real staged node/search writes precede final rejection"
    );
    assert!(matches!(result, Err(StoreError::Conflict(_))));
    assert_eq!(store.staging_node_count("staging-job:1").unwrap(), 0);
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM scan_staging_search", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn valid_checks_commit_the_original_owned_revision_and_release_staging() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let input = graph("valid", 100);
    let locations = locators(&input);
    let mut staging_checks = 0;
    store
        .append_staging_observed_iter_checked(
            "valid-job:1",
            input
                .nodes
                .iter()
                .zip(&locations)
                .map(|(node, location)| (node, location, None, None, None)),
            || {
                staging_checks += 1;
                Ok(())
            },
        )
        .unwrap();
    let mut publish_checks = 0;
    store
        .publish_revision_owned_with_batch_checked(
            "valid-job:1",
            &input,
            ("valid-revision", 1),
            Some(("server", "scope")),
            None,
            || {
                publish_checks += 1;
                Ok(())
            },
        )
        .unwrap();
    assert!(staging_checks >= 6);
    assert!(publish_checks >= 4);
    assert_eq!(
        store.revision_ownership("valid-revision").unwrap(),
        Some(("server".into(), "scope".into()))
    );
    assert_eq!(store.load_revision("valid-revision").unwrap(), input);
    assert_eq!(store.staging_node_count("valid-job:1").unwrap(), 0);
}
