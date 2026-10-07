//! 管理员授权的实时精确匹配与损坏数据拒绝回归。
use crate::relation_request_tests::published_authorization_fixture;
use crate::{EngineError, admin_scope};
use diskgraph_core::{BusinessError, Permission};

#[test]
fn admin_lookup_observes_independent_revocation_and_corrupt_text() {
    for corrupt in [false, true] {
        let (dir, engine, principal, _, _) = published_authorization_fixture();
        let policy = engine.policy_authorizer().unwrap();
        engine
            .require(&policy, &principal, &Permission::ScopeAdmin, &admin_scope())
            .unwrap();
        let independent =
            rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
        if corrupt {
            independent
                .execute_batch(
                    "INSERT INTO grants VALUES(CAST(X'FF' AS TEXT),'scope:admin','unrelated',1);",
                )
                .unwrap();
        } else {
            independent
                .execute(
                    "DELETE FROM grants WHERE principal_id=?1 AND permission=?2 AND scope_id=?3",
                    rusqlite::params![
                        principal.as_str(),
                        Permission::ScopeAdmin.wire_name(),
                        admin_scope().as_str()
                    ],
                )
                .unwrap();
        }
        let result = engine.require(&policy, &principal, &Permission::ScopeAdmin, &admin_scope());
        if corrupt {
            assert!(matches!(result, Err(EngineError::Store(_))));
        } else {
            assert!(matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ));
        }
    }
}
