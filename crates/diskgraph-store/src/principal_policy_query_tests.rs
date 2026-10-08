//! 请求授权快照不应随无关主体授权数量增长；用 SQLite 实际 VM 步数验收。
use crate::ControlStore;
use diskgraph_core::{Authorizer, Decision, Permission, PrincipalId, ScopeId};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn fixture(unrelated: usize) -> ControlStore {
    let mut store = ControlStore::open_in_memory().unwrap();
    store.publish_policy_version(1).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO grants VALUES ('selected','metadata:read','scope',1)",
            [],
        )
        .unwrap();
    store.connection.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<?1) INSERT INTO grants SELECT 'unrelated-'||x,'metadata:read','scope',1 FROM n", [unrelated as i64]).unwrap();
    store
}

fn measured(store: &ControlStore) -> u64 {
    let steps = Arc::new(AtomicU64::new(0));
    let count = Arc::clone(&steps);
    store
        .connection
        .progress_handler(
            1,
            Some(move || {
                count.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let authorizer = store
        .authorizer_for_principal(&PrincipalId::new("selected").unwrap())
        .unwrap();
    store
        .connection
        .progress_handler(0, None::<fn() -> bool>)
        .unwrap();
    assert_eq!(
        authorizer.decide(
            &PrincipalId::new("selected").unwrap(),
            &Permission::MetadataRead,
            &ScopeId::new("scope").unwrap()
        ),
        Decision::Allowed
    );
    steps.load(Ordering::Relaxed)
}

#[test]
fn request_policy_vm_work_does_not_scale_with_unrelated_subjects() {
    let small = measured(&fixture(100));
    let large = measured(&fixture(20_000));
    eprintln!("principal policy VM steps: small={small}, large={large}");
    assert!(
        large <= small * 2 + 100,
        "one subject policy scanned unrelated grants: small={small}, large={large}"
    );
}

#[test]
fn principal_policy_preserves_epoch_revocation_and_default_deny() {
    let mut store = fixture(100);
    let selected = PrincipalId::new("selected").unwrap();
    let other = PrincipalId::new("unrelated-1").unwrap();
    let scope = ScopeId::new("scope").unwrap();
    for state in 0..3 {
        if state == 1 {
            store.publish_policy_version(2).unwrap();
        }
        if state == 2 {
            store.revoke_policy().unwrap();
        }
        let full = store.authorizer().unwrap();
        let narrow = store.authorizer_for_principal(&selected).unwrap();
        for permission in [
            Permission::MetadataRead,
            Permission::ContentRead,
            Permission::ScopeAdmin,
        ] {
            assert_eq!(
                full.decide(&selected, &permission, &scope),
                narrow.decide(&selected, &permission, &scope)
            );
        }
        assert_ne!(
            narrow.decide(&other, &Permission::MetadataRead, &scope),
            Decision::Allowed
        );
    }
}
