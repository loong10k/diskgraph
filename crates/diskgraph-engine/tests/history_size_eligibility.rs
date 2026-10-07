//! D33 通过真实发布与 Engine 公开历史 API 验证大小资格。
#[path = "query_request_budget/callback_authorizer.rs"]
mod callback_authorizer;
#[path = "query_request_budget/fixture.rs"]
mod fixture;

use callback_authorizer::CallbackAuthorizer;
use diskgraph_core::{BusinessError, DifferentReason, DiskNode, NodeKind, QueryBudget, Verdict};
use diskgraph_engine::EngineError;
use diskgraph_store::{ControlStore, SqliteSnapshotStore};
use fixture::Fixture;
use std::path::Path;
use std::time::{Duration, Instant};

fn directory_fixture() -> Fixture {
    let mut f = Fixture::new(true);
    let directory = f.directory.path().join("root/dir");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("child"), b"directory payload").unwrap();
    let job = f
        .engine
        .sync_scope(&f.scope, &f.principal, &f.policy)
        .unwrap();
    f.engine.run_job(&job.job_id, "directory-fixture").unwrap();
    f.revision = f.engine.latest_revision(&f.scope).unwrap().unwrap();
    f.snapshot = f
        .engine
        .revision_reader()
        .unwrap()
        .revision(&f.revision)
        .unwrap()
        .snapshot_id;
    f
}

fn imported_pair(
    f: &Fixture,
    target: &str,
    before: impl FnOnce(&mut DiskNode),
    after: impl FnOnce(&mut DiskNode),
) -> (String, String) {
    assert!(
        f.db.is_autocommit(),
        "import must not share an uncommitted fixture transaction"
    );
    let original = f.engine.load_revision(&f.revision).unwrap();
    assert_eq!(original.snapshot.id, f.snapshot);
    let mut left = original.clone();
    let mut right = original;
    for (graph, id, offset) in [(&mut left, "before", 1), (&mut right, "after", 2)] {
        graph.snapshot.id = id.into();
        graph.snapshot.captured_at_unix_ms += offset;
    }
    let select = |node: &&mut DiskNode| {
        if target.is_empty() {
            node.parent_id.is_none()
        } else {
            node.name == target
        }
    };
    before(left.nodes.iter_mut().find(select).unwrap());
    after(right.nodes.iter_mut().find(select).unwrap());
    let mut store =
        SqliteSnapshotStore::open(&f.directory.path().join("data/diskgraph.sqlite")).unwrap();
    let owner = store.revision_ownership(&f.revision).unwrap().unwrap();
    for graph in [&left, &right] {
        store
            .append_staging_nodes(&graph.snapshot.id, &graph.nodes)
            .unwrap();
        store
            .publish_revision_owned(
                &graph.snapshot.id,
                graph,
                &graph.snapshot.id,
                graph.snapshot.captured_at_unix_ms,
                Some((&owner.0, &owner.1)),
            )
            .unwrap();
    }
    (left.snapshot.id, right.snapshot.id)
}

#[test]
fn growth_refuses_unknown_or_read_error_on_either_side_in_both_public_apis() {
    for left_bad in [false, true] {
        for read_error in [false, true] {
            let f = Fixture::new(true);
            let mutate = |node: &mut DiskNode| {
                node.subtree_bytes = 99_999;
                if read_error {
                    node.read_error = true
                } else {
                    node.size_known = false
                }
            };
            let (left, right) = imported_pair(
                &f,
                "a",
                |node| {
                    if left_bad {
                        mutate(node)
                    }
                },
                |node| {
                    if !left_bad {
                        mutate(node)
                    }
                },
            );
            assert!(
                f.engine
                    .growth_between(&left, &right, Path::new("a"))
                    .unwrap()
                    .is_none(),
                "left_bad={left_bad},read_error={read_error}"
            );
            assert!(
                f.engine
                    .growth_between_until(
                        &left,
                        &right,
                        Path::new("a"),
                        QueryBudget::default(),
                        &f.principal,
                        &f.policy,
                        Instant::now() + Duration::from_secs(30)
                    )
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn directory_unknown_comparison_has_null_bytes_and_unknown_summary_not_same() {
    for read_error in [false, true] {
        let f = directory_fixture();
        let (left, right) = imported_pair(
            &f,
            "dir",
            |node| {
                if read_error {
                    node.read_error = true
                } else {
                    node.size_known = false
                }
            },
            |_| {},
        );
        let report = f.engine.compare_revisions(&left, &right, 0).unwrap();
        let row = report.rows.iter().find(|row| row.path == "dir").unwrap();
        assert!(matches!(
            row.verdict,
            Verdict::Different {
                reason: DifferentReason::UnknownSize
            }
        ));
        assert_eq!(row.left_bytes, None);
        assert!(row.right_bytes.is_some());
        assert_eq!(report.summary.unknown, 1);
        assert_eq!(
            report.summary.same, 3,
            "only known siblings and the directory child are Same"
        );
        let json = report.to_json(None);
        assert!(
            json["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["path"] == "dir")
                .unwrap()["left_bytes"]
                .is_null()
        );
    }
}

#[test]
fn revision_changes_excludes_unknown_and_type_replacement_from_size_changed() {
    for mutation in 0..3 {
        let f = Fixture::new(true);
        let (left, right) = imported_pair(
            &f,
            "a",
            |_| {},
            |node| {
                node.subtree_bytes = 99_999;
                match mutation {
                    0 => node.size_known = false,
                    1 => node.read_error = true,
                    _ => node.kind = NodeKind::Directory,
                }
            },
        );
        let report = f.engine.revision_changes(&left, &right).unwrap();
        assert_eq!(report["size_changed"], 0, "mutation={mutation}: {report}");
        let report = f
            .engine
            .revision_changes_until(
                &left,
                &right,
                QueryBudget::default(),
                &f.principal,
                &f.policy,
                Instant::now() + Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(report["size_changed"], 0);
    }
}

#[test]
fn growth_refuses_same_path_file_directory_replacement_in_both_directions() {
    for reversed in [false, true] {
        let f = Fixture::new(true);
        let (left, right) = imported_pair(
            &f,
            "a",
            |node| {
                if reversed {
                    node.kind = NodeKind::Directory
                }
            },
            |node| {
                if !reversed {
                    node.kind = NodeKind::Directory
                };
                node.subtree_bytes += 100
            },
        );
        assert!(
            f.engine
                .growth_between(&left, &right, Path::new("a"))
                .unwrap()
                .is_none()
        );
        assert!(
            f.engine
                .growth_between_until(
                    &left,
                    &right,
                    Path::new("a"),
                    QueryBudget::default(),
                    &f.principal,
                    &f.policy,
                    Instant::now() + Duration::from_secs(30)
                )
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn unknown_growth_still_rechecks_terminal_authorization_and_budget() {
    for tiny_budget in [false, true] {
        let f = Fixture::new(true);
        let (left, right) = imported_pair(&f, "a", |node| node.size_known = false, |_| {});
        let control = f.directory.path().join("data/diskgraph-control.sqlite");
        let scope = f.scope.clone();
        let policy = CallbackAuthorizer::new(f.policy.clone(), move |call| {
            if call == 3 {
                ControlStore::open(&control)
                    .unwrap()
                    .revoke_scope(&scope)
                    .unwrap();
            }
        });
        let budget = if tiny_budget {
            QueryBudget {
                max_nodes: 1,
                ..QueryBudget::default()
            }
        } else {
            QueryBudget::default()
        };
        let result = f.engine.growth_between_until(
            &left,
            &right,
            Path::new("a"),
            budget,
            &f.principal,
            &policy,
            Instant::now() + Duration::from_secs(30),
        );
        assert!(policy.calls.get() >= 3);
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ));
    }
}

#[test]
fn unknown_growth_rejects_late_terminal_capability() {
    let f = Fixture::new(true);
    let (left, right) = imported_pair(&f, "a", |node| node.size_known = false, |_| {});
    let deadline = Instant::now() + Duration::from_secs(30);
    let policy = CallbackAuthorizer::new(f.policy.clone(), move |call| {
        if call == 3 {
            // 原数据期限仍有效；仅能力回调超出独立 50 ms 窗口。
            std::thread::sleep(Duration::from_millis(80));
        }
    });
    let result = f.engine.growth_between_until(
        &left,
        &right,
        Path::new("a"),
        QueryBudget::default(),
        &f.principal,
        &policy,
        deadline,
    );
    assert!(policy.calls.get() >= 3);
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn comparison_unknown_side_is_null_while_known_zero_remains_zero() {
    for left_unknown in [false, true] {
        let f = Fixture::new(true);
        let (left, right) = imported_pair(
            &f,
            "a",
            |node| {
                node.subtree_bytes = 0;
                node.size_known = !left_unknown
            },
            |node| {
                node.subtree_bytes = 0;
                node.size_known = left_unknown
            },
        );
        let report = f.engine.compare_revisions(&left, &right, 0).unwrap();
        let row = report.rows.iter().find(|row| row.path == "a").unwrap();
        assert_eq!(row.left_bytes, if left_unknown { None } else { Some(0) });
        assert_eq!(row.right_bytes, if left_unknown { Some(0) } else { None });
        assert!(matches!(
            row.verdict,
            Verdict::Different {
                reason: DifferentReason::UnknownSize
            }
        ));
    }
}

#[test]
fn root_unknown_growth_and_unknown_type_replacement_never_produce_delta() {
    for read_error in [false, true] {
        let f = Fixture::new(true);
        let (left, right) = imported_pair(
            &f,
            "",
            |node| {
                if read_error {
                    node.read_error = true
                } else {
                    node.size_known = false
                }
            },
            |_| {},
        );
        assert!(
            f.engine
                .growth_between(&left, &right, Path::new(""))
                .unwrap()
                .is_none()
        );
        let f = Fixture::new(true);
        let (left, right) = imported_pair(
            &f,
            "a",
            |node| {
                if read_error {
                    node.read_error = true
                } else {
                    node.size_known = false
                }
            },
            |node| node.kind = NodeKind::Directory,
        );
        let report = f.engine.compare_revisions(&left, &right, 0).unwrap();
        let row = report.rows.iter().find(|row| row.path == "a").unwrap();
        assert!(matches!(
            row.verdict,
            Verdict::Different {
                reason: DifferentReason::UnknownSize
            }
        ));
        assert_eq!(row.left_bytes, None);
        assert!(row.right_bytes.is_some());
    }
}

#[test]
fn known_directory_size_changes_count_but_count_only_changes_do_not() {
    for bytes_changed in [false, true] {
        let f = directory_fixture();
        let (left, right) = imported_pair(
            &f,
            "dir",
            |_| {},
            |node| {
                if bytes_changed {
                    node.subtree_bytes += 100
                } else {
                    node.files += 1
                }
            },
        );
        let report = f.engine.revision_changes(&left, &right).unwrap();
        assert_eq!(report["size_changed"], usize::from(bytes_changed));
        let growth = f
            .engine
            .growth_between(&left, &right, Path::new("dir"))
            .unwrap()
            .unwrap();
        assert_eq!(growth.delta_bytes, if bytes_changed { 100 } else { 0 });
    }
}
