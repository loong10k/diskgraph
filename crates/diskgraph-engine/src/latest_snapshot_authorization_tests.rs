//! 最新快照窄读的终检授权回归：成功结果与编码期间真实撤权分别验收。
use crate::EngineError;
use diskgraph_core::{BusinessError, Locator, QueryBudget};
use std::time::{Duration, Instant};

#[test]
fn latest_snapshot_encoding_still_requires_live_scope_authorization() {
    for revoke in [false, true] {
        let (dir, engine, principal, scope, _) =
            crate::relation_request_tests::published_authorization_fixture();
        let root = Locator::from_native_path(&std::fs::canonicalize(dir.path()).unwrap());
        let policy = engine.policy_authorizer().unwrap();
        let result = engine.with_latest_snapshot_id_until(
            &root,
            &principal,
            &policy,
            QueryBudget::default(),
            Instant::now() + Duration::from_secs(1),
            |snapshot, _| {
                assert_eq!(snapshot, Some("expired-envelope-snapshot"));
                if revoke {
                    engine
                        .control_store()
                        .unwrap()
                        .revoke_scope(&scope)
                        .unwrap();
                }
                Ok(())
            },
        );
        if revoke {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "{result:?}"
            );
        } else {
            assert!(result.is_ok(), "{result:?}");
        }
    }
}
