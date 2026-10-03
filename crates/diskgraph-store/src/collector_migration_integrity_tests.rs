//! 真实 v9 迁移必须确认占用节点与每个历史版本的来源闭包。
use crate::SqliteSnapshotStore;
use crate::collector_protocol_tests::downgrade_to_v9;
use crate::collector_publication_tests::{base, batch};
use diskgraph_core::{EntityKind, QueryBudget, Relation, query_deadline};
use rusqlite::params;

#[test]
fn legacy_occupied_resource_requires_valid_snapshot_node_mapping() {
    for relation in [Relation::UsedByProcess, Relation::ProtectedBy] {
        for (identity, complete) in [
            ("{}", false),
            ("{\"node_id\":99999}", false),
            ("{\"node_id\":-1}", false),
            ("{\"node_id\":0}", false),
            ("{\"node_id\":18446744073709551615}", false),
            ("{\"node_id\":2}", true),
        ] {
            let mut store = base();
            let mut observation = batch("occupied", "snap");
            observation.entities[1].kind = if relation == Relation::UsedByProcess {
                EntityKind::Process
            } else {
                EntityKind::ProtectionPolicy
            };
            observation.edges[0].relation = relation;
            observation.run.coverage_complete = false;
            observation.evidence[0].expires_at_unix_ms = Some(2);
            let budget = QueryBudget::default();
            assert_eq!(
                store
                    .candidate_selection_for_revision_until(
                        "base",
                        100,
                        budget,
                        query_deadline(budget).unwrap()
                    )
                    .unwrap()
                    .candidates
                    .len(),
                1
            );
            store
                .publish_collector_revision(
                    "base",
                    "occupied",
                    2,
                    ("server", "scope"),
                    &observation,
                    &[("occupied", "active")],
                )
                .unwrap();
            downgrade_to_v9(&store.connection);
            let mut resource = observation.entities[0].clone();
            resource.identity = identity.into();
            store.connection.execute("UPDATE entities SET entity_json=?1 WHERE snapshot_id='snap' AND entity_id='resource'",
            [serde_json::to_string(&resource).unwrap()]).unwrap();
            let store = SqliteSnapshotStore::initialize(store.connection).unwrap();
            let reader = store.revision_evidence("occupied").unwrap();
            assert_eq!(
                !reader.has_incomplete_membership().unwrap(),
                complete,
                "legacy occupancy mapping {identity}"
            );
            let budget = QueryBudget::default();
            let candidates = store.candidate_selection_for_revision_until(
                "occupied",
                100,
                budget,
                query_deadline(budget).unwrap(),
            );
            assert_eq!(
                candidates.is_ok(),
                complete,
                "candidate accepted bad mapping {identity}"
            );
            if let Ok(selection) = candidates {
                assert!(selection.candidates.is_empty());
            }
        }
    }
}

#[test]
fn legacy_revision_must_select_edge_endpoint_sources_and_valid_roles() {
    for (role, include_upstream, complete) in [
        ("active", false, false),
        ("active", true, true),
        ("invalid", true, false),
    ] {
        let mut store = base();
        let first = batch("a", "snap");
        store
            .publish_collector_revision(
                "base",
                "a",
                2,
                ("server", "scope"),
                &first,
                &[("a", "active")],
            )
            .unwrap();
        let mut second = batch("b", "snap");
        second.entities[0] = first.entities[0].clone();
        store
            .publish_collector_revision(
                "a",
                "b",
                3,
                ("server", "scope"),
                &second,
                &[("b", "active"), ("a", "dependency_only")],
            )
            .unwrap();
        downgrade_to_v9(&store.connection);
        if !include_upstream {
            store
                .connection
                .execute(
                    "DELETE FROM revision_runs WHERE revision_id='b' AND run_id='a'",
                    [],
                )
                .unwrap();
        }
        store
            .connection
            .execute(
                "UPDATE revision_runs SET role=?1 WHERE revision_id='b' AND run_id='b'",
                [role],
            )
            .unwrap();
        let store = SqliteSnapshotStore::initialize(store.connection).unwrap();
        assert!(
            !store
                .revision_evidence("a")
                .unwrap()
                .has_incomplete_membership()
                .unwrap(),
            "valid original revision lost its complete status: role={role} upstream={include_upstream}"
        );
        assert_eq!(
            !store
                .revision_evidence("b")
                .unwrap()
                .has_incomplete_membership()
                .unwrap(),
            complete,
            "legacy selection role={role} upstream={include_upstream}"
        );
        if complete {
            assert_eq!(
                store
                    .revision_evidence("b")
                    .unwrap()
                    .all_edges()
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}

#[test]
fn legacy_revision_rejects_selected_run_from_another_snapshot() {
    let mut store = base();
    let first = batch("a", "snap");
    store
        .publish_collector_revision(
            "base",
            "a",
            2,
            ("server", "scope"),
            &first,
            &[("a", "active")],
        )
        .unwrap();
    store
        .publish_revision(
            "foreign-job",
            &crate::tests::graph("foreign", 200),
            "foreign",
            4,
        )
        .unwrap();
    let foreign = batch("foreign", "foreign");
    store
        .record_collector_batch(
            "foreign",
            &foreign.run,
            &foreign.entities,
            &foreign.evidence,
            &foreign.edges,
        )
        .unwrap();
    downgrade_to_v9(&store.connection);
    store
        .connection
        .execute(
            "INSERT INTO revision_runs VALUES ('a',?1,'active')",
            params![foreign.run.run_id],
        )
        .unwrap();
    let store = SqliteSnapshotStore::initialize(store.connection).unwrap();
    assert!(
        store
            .revision_evidence("a")
            .unwrap()
            .has_incomplete_membership()
            .unwrap()
    );
}

#[test]
fn legacy_three_generation_sources_require_explicit_upstream_selection() {
    for include_upstream in [true, false] {
        let mut store = base();
        let a = batch("a", "snap");
        store
            .publish_collector_revision("base", "a", 2, ("server", "scope"), &a, &[("a", "active")])
            .unwrap();
        let mut b = batch("b", "snap");
        b.entities[0] = a.entities[0].clone();
        store
            .publish_collector_revision(
                "a",
                "b",
                3,
                ("server", "scope"),
                &b,
                &[("b", "active"), ("a", "dependency_only")],
            )
            .unwrap();
        let mut c = batch("c", "snap");
        c.entities[0] = a.entities[0].clone();
        store
            .publish_collector_revision(
                "b",
                "c",
                4,
                ("server", "scope"),
                &c,
                &[
                    ("c", "active"),
                    ("b", "dependency_only"),
                    ("a", "dependency_only"),
                ],
            )
            .unwrap();
        downgrade_to_v9(&store.connection);
        if !include_upstream {
            store
                .connection
                .execute(
                    "DELETE FROM revision_runs WHERE revision_id='c' AND run_id='a'",
                    [],
                )
                .unwrap();
        }
        let store = SqliteSnapshotStore::initialize(store.connection).unwrap();
        assert_eq!(
            !store
                .revision_evidence("c")
                .unwrap()
                .has_incomplete_membership()
                .unwrap(),
            include_upstream
        );
        assert!(
            !store
                .revision_evidence("b")
                .unwrap()
                .has_incomplete_membership()
                .unwrap()
        );
        if include_upstream {
            let edges = store.revision_evidence("c").unwrap().all_edges().unwrap();
            assert_eq!(edges.len(), 1);
            assert_eq!(edges[0].edge_id, "edge-c");
        }
    }
}
