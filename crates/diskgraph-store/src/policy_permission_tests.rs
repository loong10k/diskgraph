//! 管理员单项权限的版本、撤销及损坏字段兼容回归。
use crate::ControlStore;
use diskgraph_core::{Authorizer, Decision, Permission, PrincipalId, ScopeId};

#[test]
fn lookup_matches_legacy_policy_versions_and_exact_grants() {
    let store = ControlStore::open_in_memory().unwrap();
    let principal = PrincipalId::new("actor").unwrap();
    let scope = ScopeId::new("admin").unwrap();
    assert_eq!(
        store
            .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
            .unwrap(),
        None
    );
    store.connection.execute_batch("INSERT INTO grants VALUES ('actor','scope:admin','admin',1), ('actor','scope:admin','admin',2);").unwrap();
    for version in [-1, 0, 1, 2, 3] {
        for revoked in [0, 1] {
            store
                .connection
                .execute(
                    "INSERT OR REPLACE INTO policy VALUES (1,?1,?2)",
                    rusqlite::params![version, revoked],
                )
                .unwrap();
            for actor in ["actor", "other"] {
                let principal = PrincipalId::new(actor).unwrap();
                let expected = matches!(
                    store
                        .authorizer()
                        .unwrap()
                        .decide(&principal, &Permission::ScopeAdmin, &scope),
                    Decision::Allowed
                );
                assert_eq!(
                    store
                        .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
                        .unwrap(),
                    Some(expected)
                );
            }
        }
    }
}

#[test]
fn unrelated_corrupt_grant_remains_an_error() {
    let store = ControlStore::open_in_memory().unwrap();
    store.connection.execute_batch("INSERT INTO policy VALUES (1,1,0); INSERT INTO grants VALUES ('actor','scope:admin','admin',1); INSERT INTO grants VALUES (X'FF','scope:admin','other',1);").unwrap();
    let principal = PrincipalId::new("actor").unwrap();
    let scope = ScopeId::new("admin").unwrap();
    assert!(store.authorizer().is_err());
    assert!(
        store
            .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
            .is_err()
    );
    store
        .connection
        .execute("UPDATE policy SET revoked=1", [])
        .unwrap();
    assert_eq!(
        store
            .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
            .unwrap(),
        Some(false)
    );
}

#[test]
fn independent_connection_revocation_is_observed_without_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let store = ControlStore::open(&path).unwrap();
    store.connection.execute_batch("INSERT INTO policy VALUES(1,1,0); INSERT INTO grants VALUES('actor','scope:admin','admin',1);").unwrap();
    let principal = PrincipalId::new("actor").unwrap();
    let scope = ScopeId::new("admin").unwrap();
    assert_eq!(
        store
            .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
            .unwrap(),
        Some(true)
    );
    let independent = rusqlite::Connection::open(path).unwrap();
    independent
        .execute("UPDATE policy SET revoked=1", [])
        .unwrap();
    assert_eq!(
        store
            .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
            .unwrap(),
        Some(false)
    );
}

#[test]
#[ignore = "manual release diagnostic; not a production performance gate"]
fn paired_large_policy_lookup_diagnostic() {
    let store = ControlStore::open_in_memory().unwrap();
    store
        .connection
        .execute_batch(
            "INSERT INTO policy VALUES(1,1,0);
        INSERT INTO grants VALUES('actor','scope:admin','admin',1);
        WITH RECURSIVE seq(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM seq WHERE x<20000)
        INSERT INTO grants SELECT printf('noise-%06d',x),'metadata:read','fixture',1 FROM seq;",
        )
        .unwrap();
    let principal = PrincipalId::new("actor").unwrap();
    let scope = ScopeId::new("admin").unwrap();
    assert!(matches!(
        store
            .authorizer()
            .unwrap()
            .decide(&principal, &Permission::ScopeAdmin, &scope),
        Decision::Allowed
    ));
    assert_eq!(
        store
            .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
            .unwrap(),
        Some(true)
    );
    let mut legacy = Vec::new();
    let mut candidate = Vec::new();
    for round in 0..40 {
        for narrow in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let start = std::time::Instant::now();
            if narrow {
                std::hint::black_box(
                    store
                        .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
                        .unwrap(),
                );
            } else {
                std::hint::black_box(store.authorizer().unwrap().decide(
                    &principal,
                    &Permission::ScopeAdmin,
                    &scope,
                ));
            }
            if narrow {
                candidate.push(start.elapsed().as_nanos());
            } else {
                legacy.push(start.elapsed().as_nanos());
            }
        }
    }
    legacy.sort_unstable();
    candidate.sort_unstable();
    println!(
        "ADMIN_PERMISSION_BENCH {}",
        serde_json::json!({"grants":20001,"pairs":40,
        "legacy_p50_ns":legacy[19],"legacy_p95_ns":legacy[37],
        "candidate_p50_ns":candidate[19],"candidate_p95_ns":candidate[37],
        "full_type_validation":true,"database":"isolated in-memory","warm_cache":true})
    );
}

#[test]
fn unrelated_invalid_utf8_text_remains_an_error() {
    let store = ControlStore::open_in_memory().unwrap();
    store.connection.execute_batch("INSERT INTO policy VALUES(1,1,0); INSERT INTO grants VALUES('actor','scope:admin','admin',1); INSERT INTO grants VALUES(CAST(X'FF' AS TEXT),'scope:admin','other',1);").unwrap();
    let principal = PrincipalId::new("actor").unwrap();
    let scope = ScopeId::new("admin").unwrap();
    assert!(store.authorizer().is_err());
    assert!(
        store
            .policy_permission(&principal, &Permission::ScopeAdmin, &scope)
            .is_err()
    );
}
